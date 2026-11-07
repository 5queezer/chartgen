//! What one connection may cost (ADR-0051). A client that asks for something and never
//! reads it leaves the answer in the kernel's send buffer, charged to this process's
//! cgroup, and the handler has long finished. So the bound belongs here, for every route
//! and every client, signed in or not: a small buffer per socket, so many sockets at
//! once, and an end to a socket that has stopped moving.

use axum::body::{Body, HttpBody};
use axum::http::header;
use axum::response::Response;
use axum::serve::Listener;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::task::{ready, Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{sleep, Instant, Sleep};

/// Connections open at once. Past it, an accept waits: the client queues rather than fails.
///
/// A connection whose client has stopped reading costs up to `SEND_BUFFER * 2` in the
/// kernel (Linux doubles what is asked) and about 470 KiB queued in hyper, measured, so
/// about 730 KiB: 93 MiB for all of them, against a 256 MiB container. One whose request
/// head never ends costs less. Behind a proxy a few connections carry many users.
const MAX_CONNECTIONS: usize = 128;
/// Room for answers on their way, enough to keep a connection to a proxy or a browser
/// moving, and little enough that many of them cannot fill the container.
const SEND_BUFFER: u32 = 128 * 1024;
/// Asked for at all, because the kernel would otherwise grow it to several MiB.
const RECV_BUFFER: u32 = 48 * 1024;
/// The queue of connections waiting for an accept.
const BACKLOG: u32 = 1024;

/// Long-lived streams, so that they alone cannot take every connection and the rest of
/// the app stays reachable. The same as the chart subscriptions the hub allows in all. A
/// tab refused one goes on polling.
const MAX_STREAMS: usize = 64;

// With about 470 KiB queued in hyper for each, measured.
const _: () =
    assert!(MAX_CONNECTIONS * (SEND_BUFFER as usize * 2 + 470 * 1024) < 100 * 1024 * 1024);
const _: () = assert!(MAX_STREAMS < MAX_CONNECTIONS);

static STREAMS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(MAX_STREAMS)));
static STREAM_PLACES: LazyLock<Mutex<HashMap<String, Weak<OwnedSemaphorePermit>>>> =
    LazyLock::new(Default::default);

/// A place for one long-lived response, held for as long as it runs; `None` when all are
/// taken. A session's new stream shares the place of the one it replaces, so a client whose
/// connection died can come back with the cap full of others.
pub(super) fn stream_slot(session: &str) -> Option<Arc<OwnedSemaphorePermit>> {
    let mut places = STREAM_PLACES.lock().unwrap();
    if let Some(place) = places.get(session).and_then(Weak::upgrade) {
        return Some(place);
    }
    places.retain(|_, place| place.strong_count() > 0);
    let place = Arc::new(STREAMS.clone().try_acquire_owned().ok()?);
    places.insert(session.to_owned(), Arc::downgrade(&place));
    Some(place)
}

/// What tool calls may hold at once, in bars times series, from the moment they run until
/// their answer has gone out. A chart is built in memory, measured at 0.8 to 1.5 KiB per
/// such unit, and for a client that does not read, hyper keeps the answer. The rest wait,
/// up to `CALL_WAIT`, and a call larger than all of it is refused. Under a flood of large
/// charts with every other connection held, this peaks at about 130 MiB in a 256 MiB
/// container, where 48,000 reaches 216 and 64,000 the limit. It fits the app's own use: 500
/// bars a call, so a 4-chart grid at 10 indicators each is 22,000. And 50 indicators at
/// 500 bars, the most it asks for, is 25,500.
const CALL_BUDGET: u32 = 32_000;
/// A call that has waited this long is told to come back, rather than holding its
/// connection until something gives up on it.
const CALL_WAIT: Duration = Duration::from_secs(20);

static CALLS: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(CALL_BUDGET as usize)));

pub(super) enum NoRoom {
    /// More than the budget could ever hold: it will never fit, however long it waits.
    TooLarge { cost: u32, budget: u32 },
    /// Others hold the budget. Come back in this many seconds.
    Busy { retry_after: u64 },
}

/// A place for one tool call of this cost; pass it to `held_until_written`.
pub(super) async fn call_slot(cost: u32) -> Result<OwnedSemaphorePermit, NoRoom> {
    if cost > CALL_BUDGET {
        return Err(NoRoom::TooLarge {
            cost,
            budget: CALL_BUDGET,
        });
    }
    match tokio::time::timeout(CALL_WAIT, CALLS.clone().acquire_many_owned(cost)).await {
        Ok(permit) => Ok(permit.expect("the semaphore is never closed")),
        Err(_) => Err(NoRoom::Busy { retry_after: 5 }),
    }
}

/// What a request to `/mcp` will hold, in the units of `CALL_BUDGET`. Only charts come to
/// much, and what is no tool call costs nothing, so that it never waits. A price-based chart reads up to `fetch::MAX_BASE_BARS` to build its bricks; the
/// indicators then run on the bricks.
pub(super) fn call_cost(body: &[u8]) -> u32 {
    const SMALL: u64 = 1_000;
    const DEFAULT_BARS: u64 = 200;
    /// `mcp::indicators_arg`'s default.
    const DEFAULT_INDICATORS: u64 = 3;
    const MAX_BARS: u64 = 5_000;
    const SOURCE_BARS: u64 = 20_000;
    const PRICE_BASED: [&str; 5] = ["renko", "kagi", "point_figure", "line_break", "range"];
    let Ok(call) = serde_json::from_slice::<serde_json::Value>(body) else {
        return 0;
    };
    if call["method"] != "tools/call" {
        return 0;
    }
    let params = &call["params"];
    if params["name"] != "generate_chart" {
        return SMALL as u32;
    }
    let args = &params["arguments"];
    let bars = args["bars"].as_u64().unwrap_or(DEFAULT_BARS).min(MAX_BARS);
    let indicators = match args.get("indicators").or_else(|| args.get("panels")) {
        Some(list) => list.as_array().map_or(0, Vec::len) as u64,
        None => DEFAULT_INDICATORS,
    };
    let shown = bars.saturating_mul(indicators.saturating_add(1));
    let price_based = args["chart_style"]
        .as_str()
        .is_some_and(|style| PRICE_BASED.contains(&style));
    let cost = shown + if price_based { SOURCE_BARS } else { 0 };
    u32::try_from(cost.max(SMALL)).unwrap_or(u32::MAX)
}

/// A body comes out in pieces of this, so that hyper asks for the next only when it has
/// room: what is queued for a slow client stays small, and the last piece says it is out.
pub(super) const PIECE: usize = 64 * 1024;

/// `response`, keeping `guard` until hyper has taken its last piece for writing, for a
/// response that a client does not read. The buffers below that are `MAX_CONNECTIONS`' to
/// bound: up to about 730 KiB can still be on its way after the guard has gone.
pub(super) fn held_until_written<G: Send + 'static>(response: Response, guard: G) -> Response {
    let guard = (guard, MarksFreed);
    let (mut parts, body) = response.into_parts();
    match body.size_hint().exact() {
        Some(0) => return Response::from_parts(parts, Body::empty()),
        Some(length) => {
            parts.headers.insert(header::CONTENT_LENGTH, length.into());
        }
        None => {}
    }
    let stream = async_stream::stream! {
        let mut body = body.into_data_stream();
        while let Some(chunk) = body.next().await {
            match chunk {
                Ok(mut data) => {
                    while data.len() > PIECE {
                        yield Ok(data.split_to(PIECE));
                    }
                    yield Ok(data);
                }
                Err(e) => yield Err(e),
            }
        }
        drop(guard);
    };
    Response::from_parts(parts, Body::from_stream(stream))
}

#[derive(Clone, Copy)]
struct Deadlines {
    /// A write that has not moved a byte for this long: the client has stopped reading.
    /// The kernel takes more once about a third of the buffer has gone, so this ends a
    /// client slower than about 3 KB/s too.
    write: Duration,
    /// Nothing either way for this long. Past the 90 s after which proxies drop an idle
    /// connection themselves, so that they find out first. A healthy SSE stream writes a
    /// keepalive every 30 s, and a request gets its answer well within it.
    idle: Duration,
    /// After our last byte, for the client to read it and hang up.
    linger: Duration,
}

const DEADLINES: Deadlines = Deadlines {
    write: Duration::from_secs(30),
    idle: Duration::from_secs(120),
    linger: Duration::from_secs(30),
};

/// Something large may have been freed since the last trim.
static FREED: AtomicBool = AtomicBool::new(false);

/// Marks that, when it goes.
struct MarksFreed;

impl Drop for MarksFreed {
    fn drop(&mut self) {
        FREED.store(true, Ordering::Relaxed);
    }
}

/// How often freed memory is given back, when there is any to give.
const TRIM_EVERY: Duration = Duration::from_secs(2);

/// glibc keeps what a burst of large charts has freed, per arena, and it adds up to most
/// of the container with nothing held at all (ADR-0051). Two arenas, and a trim soon after
/// something large has gone, keep that down. Elsewhere the allocator is left as it is.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub(super) fn keep_freed_memory_down() {
    // SAFETY: a plain setting, read when a thread first needs an arena.
    unsafe { libc::mallopt(libc::M_ARENA_MAX, 2) };
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(TRIM_EVERY);
        loop {
            tick.tick().await;
            if FREED.swap(false, Ordering::Relaxed) {
                // SAFETY: returns free pages to the kernel, under the allocator's own locks.
                let _ = tokio::task::spawn_blocking(|| unsafe { libc::malloc_trim(0) }).await;
            }
        }
    });
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub(super) fn keep_freed_memory_down() {}

pub(super) struct BoundedListener {
    listener: TcpListener,
    slots: Arc<Semaphore>,
    deadlines: Deadlines,
}

impl BoundedListener {
    pub(super) fn bind(addr: SocketAddr) -> io::Result<Self> {
        Self::with_limits(addr, MAX_CONNECTIONS, DEADLINES)
    }

    fn with_limits(addr: SocketAddr, max: usize, deadlines: Deadlines) -> io::Result<Self> {
        let socket = match addr {
            SocketAddr::V4(_) => TcpSocket::new_v4()?,
            SocketAddr::V6(_) => TcpSocket::new_v6()?,
        };
        // As `TcpListener::bind` does: on Windows it would let another process share the port.
        #[cfg(not(windows))]
        socket.set_reuseaddr(true)?;
        // On the listener, so a connection has them from its first byte.
        socket.set_send_buffer_size(SEND_BUFFER)?;
        socket.set_recv_buffer_size(RECV_BUFFER)?;
        socket.bind(addr)?;
        Ok(Self {
            listener: socket.listen(BACKLOG)?,
            slots: Arc::new(Semaphore::new(max)),
            deadlines,
        })
    }
}

impl Listener for BoundedListener {
    type Io = BoundedStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (BoundedStream, SocketAddr) {
        let slot = self
            .slots
            .clone()
            .acquire_owned()
            .await
            .expect("the semaphore is never closed");
        loop {
            match self.listener.accept().await {
                Ok((io, addr)) => {
                    let deadlines = self.deadlines;
                    let stream = BoundedStream {
                        io,
                        _slot: slot,
                        deadlines,
                        idle: Box::pin(sleep(deadlines.idle)),
                        stalled: None,
                        closing: None,
                        unsent_seen: None,
                        _freed: MarksFreed,
                    };
                    return (stream, addr);
                }
                // Out of file descriptors, say. It passes when others close.
                Err(e) => {
                    eprintln!("[server] accept failed: {e}");
                    sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}

/// A connection, holding its place until it closes.
pub(super) struct BoundedStream {
    io: TcpStream,
    _slot: OwnedSemaphorePermit,
    deadlines: Deadlines,
    idle: Pin<Box<Sleep>>,
    stalled: Option<Pin<Box<Sleep>>>,
    closing: Option<Closing>,
    /// `unsent` when last looked at.
    unsent_seen: Option<usize>,
    _freed: MarksFreed,
}

struct Closing {
    tick: Pin<Box<Sleep>>,
    moved_at: Instant,
}

/// What the peer has not acknowledged yet: how a connection tells a client reading slowly
/// from one that has stopped, since what the kernel holds moves with no write from here.
/// `None` where that cannot be told, and a connection is then judged by its writes alone.
#[cfg(target_os = "linux")]
fn unsent(io: &TcpStream) -> Option<usize> {
    use std::os::fd::AsRawFd;
    let mut queued: libc::c_int = 0;
    // SAFETY: the descriptor is open for as long as `io` is borrowed, and SIOCOUTQ writes
    // one int to the pointer it is given.
    let result = unsafe { libc::ioctl(io.as_raw_fd(), libc::TIOCOUTQ as _, &mut queued) };
    (result == 0).then_some(queued as usize)
}

#[cfg(not(target_os = "linux"))]
fn unsent(_: &TcpStream) -> Option<usize> {
    None
}

impl BoundedStream {
    /// Close at once, dropping what the client did not read. A plain close would leave the
    /// kernel trying to send it for minutes, as an orphan still charged to us, with the
    /// place here already free.
    fn abandon(&mut self) {
        let _ = self.io.set_zero_linger();
    }

    fn moved(&mut self) {
        self.stalled = None;
        self.unsent_seen = unsent(&self.io);
        let at = Instant::now() + self.deadlines.idle;
        self.idle.as_mut().reset(at);
    }

    /// Whether the client has taken anything of what the kernel holds since last asked.
    fn client_is_reading(&mut self) -> bool {
        let now = unsent(&self.io);
        let reading = matches!((self.unsent_seen, now), (Some(before), Some(now)) if now < before);
        self.unsent_seen = now;
        reading
    }

    /// What a write came to. A write that waits starts the clock, and one that moves stops it.
    fn after_write<T>(
        &mut self,
        cx: &mut Context<'_>,
        polled: Poll<io::Result<T>>,
    ) -> Poll<io::Result<T>> {
        match polled {
            Poll::Ready(result) => {
                self.moved();
                Poll::Ready(result)
            }
            Poll::Pending => {
                let write = self.deadlines.write;
                let stalled = self.stalled.get_or_insert_with(|| Box::pin(sleep(write)));
                if stalled.as_mut().poll(cx).is_pending() {
                    return Poll::Pending;
                }
                if !self.client_is_reading() {
                    self.abandon();
                    return Poll::Ready(Err(io::ErrorKind::TimedOut.into()));
                }
                let stalled = self.stalled.as_mut().expect("set above");
                stalled.as_mut().reset(Instant::now() + write);
                let _ = stalled.as_mut().poll(cx);
                Poll::Pending
            }
        }
    }
}

impl AsyncRead for BoundedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        match Pin::new(&mut this.io).poll_read(cx, buf) {
            Poll::Ready(result) => {
                if buf.filled().len() > before {
                    this.moved();
                }
                Poll::Ready(result)
            }
            Poll::Pending => {
                if this.idle.as_mut().poll(cx).is_pending() {
                    return Poll::Pending;
                }
                if !this.client_is_reading() {
                    this.abandon();
                    return Poll::Ready(Err(io::ErrorKind::TimedOut.into()));
                }
                let at = Instant::now() + this.deadlines.idle;
                this.idle.as_mut().reset(at);
                let _ = this.idle.as_mut().poll(cx);
                Poll::Pending
            }
        }
    }
}

impl AsyncWrite for BoundedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.io).poll_write(cx, buf);
        this.after_write(cx, polled)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.io).poll_write_vectored(cx, bufs);
        this.after_write(cx, polled)
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_flush(cx)
    }

    /// Say we are done, then keep the place until what we sent has reached the client.
    /// Closing at once with `Connection: close` would free the place with the answer still
    /// unread, so a client could have any number of them by just asking again. One still
    /// reading gets all of it. One that has stopped is reset, which drops what it did not
    /// take, as a plain close would leave the kernel sending it for minutes. Where what is
    /// unread cannot be told, it gets `linger` and then a plain close, as cutting off a slow
    /// reader is worse there than an orphan.
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let tick = this.deadlines.linger / 30;
        if this.closing.is_none() {
            ready!(Pin::new(&mut this.io).poll_shutdown(cx))?;
            this.unsent_seen = unsent(&this.io);
            this.closing = Some(Closing {
                tick: Box::pin(sleep(tick)),
                moved_at: Instant::now(),
            });
        }
        let mut discard = [0u8; 1024];
        loop {
            let mut buf = ReadBuf::new(&mut discard);
            match Pin::new(&mut this.io).poll_read(cx, &mut buf) {
                Poll::Ready(Ok(())) if buf.filled().is_empty() => return Poll::Ready(Ok(())),
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(_)) => return Poll::Ready(Ok(())),
                Poll::Pending => break,
            }
        }
        loop {
            let reading = this.client_is_reading();
            // The FIN counts as one byte until it is acknowledged.
            if this.unsent_seen.is_some_and(|left| left <= 1) {
                return Poll::Ready(Ok(()));
            }
            let can_tell = this.unsent_seen.is_some();
            let linger = this.deadlines.linger;
            let closing = this.closing.as_mut().expect("set above");
            if reading {
                closing.moved_at = Instant::now();
            }
            if closing.tick.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }
            if closing.moved_at.elapsed() >= linger {
                if can_tell {
                    this.abandon();
                }
                return Poll::Ready(Ok(()));
            }
            closing.tick.as_mut().reset(Instant::now() + tick);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const SHORT: Deadlines = Deadlines {
        write: Duration::from_millis(300),
        idle: Duration::from_millis(500),
        linger: Duration::from_millis(300),
    };

    fn listener(max: usize, deadlines: Deadlines) -> BoundedListener {
        BoundedListener::with_limits("127.0.0.1:0".parse().unwrap(), max, deadlines).unwrap()
    }

    /// A client that reads little: the kernel gives it a small window from the start.
    async fn client(addr: SocketAddr) -> TcpStream {
        let socket = TcpSocket::new_v4().unwrap();
        socket.set_recv_buffer_size(4096).unwrap();
        socket.connect(addr).await.unwrap()
    }

    async fn pair(deadlines: Deadlines) -> (BoundedStream, TcpStream, BoundedListener) {
        let mut l = listener(4, deadlines);
        let addr = l.local_addr().unwrap();
        let peer = client(addr).await;
        let (stream, _) = l.accept().await;
        (stream, peer, l)
    }

    /// No test of a limit should hang when the limit is gone.
    async fn soon<T>(future: impl Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(5), future)
            .await
            .expect("it should have ended by now")
    }

    #[tokio::test]
    async fn a_client_that_stops_reading_is_dropped_with_little_held_for_it() {
        let (mut stream, _peer, _l) = pair(SHORT).await;
        let chunk = vec![0u8; 16 * 1024];
        let mut written = 0usize;
        let started = std::time::Instant::now();
        let error = soon(async {
            loop {
                match stream.write_all(&chunk).await {
                    Ok(()) => written += chunk.len(),
                    Err(e) => break e,
                }
            }
        })
        .await;
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() >= SHORT.write, "{:?}", started.elapsed());
        assert!(started.elapsed() < Duration::from_secs(5));
        // What the two buffers hold, and well short of the MiB the kernel would allow.
        assert!(written < 512 * 1024, "{written} bytes went into the socket");
    }

    /// Only Linux doubles what is asked for.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn the_buffers_are_the_ones_asked_for() {
        let (stream, _peer, _l) = pair(SHORT).await;
        let socket = TcpSocket::from_std_stream(stream.io.into_std().unwrap());
        assert_eq!(socket.send_buffer_size().unwrap(), SEND_BUFFER * 2);
        assert_eq!(socket.recv_buffer_size().unwrap(), RECV_BUFFER * 2);
    }

    #[tokio::test]
    async fn a_slow_client_that_keeps_reading_is_not() {
        let mut l = listener(4, SHORT);
        let mut peer = TcpStream::connect(l.local_addr().unwrap()).await.unwrap();
        let (mut stream, _) = l.accept().await;
        let reader = tokio::spawn(async move {
            let mut buf = vec![0u8; 64 * 1024];
            let mut total = 0;
            loop {
                match peer.read(&mut buf).await {
                    Ok(0) | Err(_) => return total,
                    Ok(n) => total += n,
                }
                sleep(SHORT.write / 6).await;
            }
        });
        let chunk = vec![0u8; 256 * 1024];
        for _ in 0..3 {
            stream.write_all(&chunk).await.unwrap();
        }
        stream.io.shutdown().await.unwrap();
        assert_eq!(reader.await.unwrap(), 3 * chunk.len());
    }

    #[tokio::test]
    async fn a_silent_connection_ends_and_the_client_is_told_at_once() {
        let (mut stream, mut peer, _l) = pair(SHORT).await;
        let started = std::time::Instant::now();
        let mut buf = [0u8; 16];
        let error = soon(stream.read(&mut buf)).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() >= SHORT.idle, "{:?}", started.elapsed());
        drop(stream);
        let seen = soon(peer.read(&mut buf)).await;
        assert_eq!(seen.unwrap_err().kind(), io::ErrorKind::ConnectionReset);
    }

    /// A long-lived stream sits waiting to read, and writes now and then: that is not idle.
    #[tokio::test]
    async fn a_stream_with_a_keepalive_is_not_silent() {
        let (mut stream, mut peer, _l) = pair(SHORT).await;
        let reader = tokio::spawn(async move {
            let mut buf = [0u8; 64];
            while peer.read(&mut buf).await.is_ok_and(|n| n > 0) {}
        });
        let until = Instant::now() + SHORT.idle * 3;
        let mut tick = tokio::time::interval(SHORT.idle / 3);
        let mut buf = [0u8; 16];
        while Instant::now() < until {
            tokio::select! {
                read = stream.read(&mut buf) => panic!("the connection ended: {read:?}"),
                _ = tick.tick() => stream.write_all(b": keepalive\n\n").await.unwrap(),
            }
        }
        reader.abort();
    }

    #[tokio::test]
    async fn a_request_that_goes_on_arriving_is_not_silent() {
        let (mut stream, mut peer, _l) = pair(SHORT).await;
        let writer = tokio::spawn(async move {
            for _ in 0..9 {
                peer.write_all(b"x").await.unwrap();
                sleep(SHORT.idle / 3).await;
            }
        });
        let mut got = 0;
        let mut buf = [0u8; 16];
        while got < 9 {
            got += stream.read(&mut buf).await.unwrap();
        }
        writer.await.unwrap();
    }

    /// Reads a little at a time, then what it got and how that ended.
    async fn read_slowly(
        mut peer: TcpStream,
        chunk: usize,
        pause: Duration,
    ) -> (usize, io::Result<()>) {
        let mut buf = vec![0u8; chunk];
        let mut total = 0;
        loop {
            match peer.read(&mut buf).await {
                Ok(0) => return (total, Ok(())),
                Ok(n) => total += n,
                Err(e) => return (total, Err(e)),
            }
            sleep(pause).await;
        }
    }

    /// Where what is unread cannot be told, a close takes its `linger`.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn closing_with_nothing_unread_takes_no_time() {
        let (mut stream, mut peer, _l) = pair(Deadlines {
            linger: Duration::from_secs(30),
            ..SHORT
        })
        .await;
        stream.write_all(b"the answer").await.unwrap();
        let mut got = [0u8; 10];
        peer.read_exact(&mut got).await.unwrap();
        // The client has it all and has not hung up: there is nothing to wait for.
        soon(stream.shutdown()).await.unwrap();
        assert_eq!(&got, b"the answer");
    }

    /// The answer fits the kernel's buffer, so from here it is written the moment it is
    /// handed over, and the client takes far longer to read it than `linger`.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn closing_does_not_cut_a_client_still_reading() {
        let (mut stream, peer, _l) = pair(SHORT).await;
        let answer = vec![1u8; 128 * 1024];
        stream.write_all(&answer).await.unwrap();
        let reader = tokio::spawn(read_slowly(peer, 4096, SHORT.linger / 10));
        let started = std::time::Instant::now();
        stream.shutdown().await.unwrap();
        drop(stream);
        let (total, ended) = reader.await.unwrap();
        assert!(started.elapsed() > SHORT.linger, "{:?}", started.elapsed());
        assert_eq!(total, answer.len());
        ended.unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn closing_gives_up_on_a_client_that_has_stopped_reading() {
        let (mut stream, mut peer, _l) = pair(SHORT).await;
        stream.write_all(&vec![0u8; 128 * 1024]).await.unwrap();
        let started = std::time::Instant::now();
        soon(stream.shutdown()).await.unwrap();
        assert!(started.elapsed() >= SHORT.linger, "{:?}", started.elapsed());
        drop(stream);
        // What it did not read is gone, not left to the kernel to go on sending.
        let mut sink = Vec::new();
        assert_eq!(
            soon(peer.read_to_end(&mut sink)).await.unwrap_err().kind(),
            io::ErrorKind::ConnectionReset
        );
    }

    /// Slower than the kernel asks for more: what moves is what it holds.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_write_waiting_on_a_slow_client_is_not_a_stalled_one() {
        let (mut stream, peer, _l) = pair(SHORT).await;
        let reader = tokio::spawn(read_slowly(peer, 2048, SHORT.write / 10));
        let answer = vec![1u8; 400 * 1024];
        stream.write_all(&answer).await.unwrap();
        stream.shutdown().await.unwrap();
        drop(stream);
        let (total, ended) = reader.await.unwrap();
        assert_eq!(total, answer.len());
        ended.unwrap();
    }

    /// Nothing either way from here, while the client works through what it was sent.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_client_reading_what_it_was_sent_is_not_silent() {
        let (mut stream, peer, _l) = pair(SHORT).await;
        let answer = vec![1u8; 128 * 1024];
        stream.write_all(&answer).await.unwrap();
        let reader = tokio::spawn(read_slowly(peer, 2048, SHORT.idle / 10));
        let mut buf = [0u8; 16];
        let until = Instant::now() + SHORT.idle * 3;
        tokio::select! {
            read = stream.read(&mut buf) => panic!("the connection ended: {read:?}"),
            _ = tokio::time::sleep_until(until) => {}
        }
        reader.abort();
    }

    #[tokio::test]
    async fn past_the_cap_an_accept_waits_and_gets_its_turn() {
        let mut l = listener(2, SHORT);
        let addr = l.local_addr().unwrap();
        let _a = client(addr).await;
        let _b = client(addr).await;
        let _c = client(addr).await;
        let (first, _) = l.accept().await;
        let (_second, _) = l.accept().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(200), l.accept())
                .await
                .is_err(),
            "the third waits"
        );
        drop(first);
        tokio::time::timeout(Duration::from_secs(2), l.accept())
            .await
            .expect("and comes in when one closes");
    }

    fn chart_cost(args: serde_json::Value) -> u32 {
        call_cost(
            serde_json::json!({
                "method": "tools/call",
                "params": { "name": "generate_chart", "arguments": args },
            })
            .to_string()
            .as_bytes(),
        )
    }

    #[test]
    fn a_call_costs_what_it_asks_for() {
        // What the app sends: `web/src/queries.ts`, `ChartPane.tsx` and `mcp.ts`.
        let usual = serde_json::json!({
            "ticker": "BTCUSDT", "timeframe": "1h", "format": "series", "bars": 500,
            "warmup": 300, "chart_style": "candles", "indicators": ["rsi", "macd", "ema"],
        });
        assert_eq!(chart_cost(usual), 2_000, "the app's usual chart");
        let tail =
            serde_json::json!({ "bars": 60, "chart_style": "candles", "indicators": ["rsi"] });
        assert_eq!(chart_cost(tail), 1_000, "and its tail poll");

        for style in ["candles", "heikin_ashi"] {
            let chart = serde_json::json!({ "bars": 5000, "chart_style": style, "indicators": ["a", "b", "c"] });
            assert_eq!(chart_cost(chart), 20_000, "{style} is built bar for bar");
        }
        for style in ["renko", "kagi", "point_figure", "line_break", "range"] {
            let chart =
                serde_json::json!({ "bars": 500, "chart_style": style, "indicators": ["a"] });
            assert_eq!(chart_cost(chart), 21_000, "{style} reads the source window");
        }
        assert_eq!(
            chart_cost(serde_json::json!({})),
            1_000,
            "200 bars, 3 by default"
        );
        assert_eq!(chart_cost(serde_json::json!({ "bars": 1000 })), 4_000);
        assert_eq!(
            chart_cost(
                serde_json::json!({ "bars": 5000, "indicators": ["a", "b", "c", "d", "e", "f"] })
            ),
            35_000
        );
        assert_eq!(
            chart_cost(
                serde_json::json!({ "bars": 5000, "panels": ["a", "b", "c", "d", "e", "f"] })
            ),
            35_000,
            "`panels` is another name for them"
        );
        assert_eq!(
            chart_cost(serde_json::json!({ "bars": 99_999_999, "indicators": vec!["a"; 50] })),
            255_000,
            "bars stop at the tool's own limit"
        );
        assert_eq!(call_cost(b"junk"), 0, "answered without running anything");
        assert_eq!(call_cost(br#"{"method":"tools/list"}"#), 0);
        assert_eq!(
            call_cost(br#"{"method":"tools/call","params":{"name":"get_x"}}"#),
            1_000
        );
    }

    #[tokio::test]
    async fn a_call_larger_than_the_budget_is_told_so_at_once() {
        let refused = soon(call_slot(CALL_BUDGET + 1)).await;
        assert!(
            matches!(refused, Err(NoRoom::TooLarge { .. })),
            "it would never fit"
        );
    }

    #[tokio::test]
    async fn a_large_call_waits_for_room_and_small_ones_share() {
        let budget = || Arc::new(Semaphore::new(CALL_BUDGET as usize));
        let calls = budget();
        let large = calls
            .clone()
            .acquire_many_owned(CALL_BUDGET - 500)
            .await
            .unwrap();
        assert!(calls.clone().try_acquire_many_owned(1_000).is_err());
        drop(large);
        // A grid of four, at the most indicators a usual chart has, and one more.
        let grid = chart_cost(serde_json::json!({
            "bars": 500, "chart_style": "candles", "indicators": vec!["a"; 10],
        }));
        assert_eq!(grid, 5_500);
        let charts: Vec<_> = (0..5)
            .map(|_| calls.clone().try_acquire_many_owned(grid).unwrap())
            .collect();
        assert_eq!(
            charts.len(),
            5,
            "the app's charts do not wait on each other"
        );
    }

    #[tokio::test]
    async fn an_answer_keeps_its_guard_until_its_last_piece_is_taken() {
        let guard = Arc::new(());
        let answer = vec![7u8; PIECE * 2 + 1];
        let response = held_until_written(Response::new(Body::from(answer.clone())), guard.clone());
        assert_eq!(
            response.headers()[header::CONTENT_LENGTH],
            answer.len().to_string(),
            "keeps its length"
        );
        let mut body = response.into_body().into_data_stream();
        let mut got = Vec::new();
        for expected in [PIECE, PIECE, 1] {
            assert_eq!(Arc::strong_count(&guard), 2, "not yet");
            let piece = body.next().await.unwrap().unwrap();
            assert_eq!(piece.len(), expected);
            got.extend_from_slice(&piece);
        }
        assert_eq!(
            Arc::strong_count(&guard),
            2,
            "nor with the last piece just handed over"
        );
        assert!(body.next().await.is_none());
        assert_eq!(Arc::strong_count(&guard), 1);
        assert_eq!(got, answer);
    }

    #[tokio::test]
    async fn a_client_hanging_up_lets_the_guard_go() {
        let guard = Arc::new(());
        let response = held_until_written(Response::new(Body::from(vec![0u8; 8])), guard.clone());
        drop(response);
        assert_eq!(Arc::strong_count(&guard), 1);
    }

    #[tokio::test]
    async fn an_empty_answer_needs_no_guard_and_stays_empty() {
        let guard = Arc::new(());
        let response = held_until_written(Response::new(Body::empty()), guard.clone());
        assert_eq!(Arc::strong_count(&guard), 1);
        assert!(response.into_body().is_end_stream());
    }
}
