import './style.css';

import { getAccessToken } from './oauth.js';
import { openMcpSession, type McpSession } from './mcp.js';
import { createKlineChart, type KlineController } from './chart.js';

const BASE_URL =
  import.meta.env.VITE_CHARTGEN_BASE_URL ?? window.location.origin;

const els = {
  form: document.getElementById('chart-form') as HTMLFormElement,
  ticker: document.getElementById('ticker') as HTMLInputElement,
  timeframe: document.getElementById('timeframe') as HTMLSelectElement,
  loadBtn: document.getElementById('load-btn') as HTMLButtonElement,
  status: document.getElementById('status') as HTMLSpanElement,
  chart: document.getElementById('chart') as HTMLDivElement,
};

function setStatus(text: string, isError = false): void {
  els.status.textContent = text;
  els.status.classList.toggle('err', isError);
}

// --- App state ---------------------------------------------------------------

let session: McpSession | null = null;
let chart: KlineController | null = null;
let accessToken: string | null = null;

async function ensureSession(opts: { forceReauth?: boolean } = {}): Promise<McpSession> {
  if (session && !opts.forceReauth) return session;
  if (session) {
    await session.close().catch(() => {});
    session = null;
  }
  if (opts.forceReauth) {
    accessToken = null;
  }
  if (!accessToken) {
    setStatus('authorizing…');
    accessToken = await getAccessToken({ baseUrl: BASE_URL }, opts);
  }
  setStatus('connecting…');
  session = await openMcpSession({
    baseUrl: BASE_URL,
    accessToken,
    onNotification: (n) => {
      console.log('notifications/alert_triggered', n);
    },
    // 401s surface as thrown errors from callGenerateChart; the caller in
    // load() catches them and re-runs the full auth flow.
  });
  return session;
}

async function load(): Promise<void> {
  const ticker = els.ticker.value.trim().toUpperCase();
  const timeframe = els.timeframe.value;
  if (!ticker) return;

  els.loadBtn.disabled = true;
  setStatus(`loading ${ticker} ${timeframe}…`);

  try {
    let s = await ensureSession();
    let payload;
    try {
      payload = await s.callGenerateChart({ ticker, timeframe });
    } catch (err) {
      if (is401(err)) {
        // Token expired mid-session; restart everything once.
        s = await ensureSession({ forceReauth: true });
        payload = await s.callGenerateChart({ ticker, timeframe });
      } else {
        throw err;
      }
    }

    if (!chart) chart = createKlineChart(els.chart);
    chart.render(payload);

    setStatus(`${payload.bars.length} bars · ${ticker} ${timeframe}`);
  } catch (err) {
    console.error(err);
    setStatus(err instanceof Error ? err.message : 'error', true);
  } finally {
    els.loadBtn.disabled = false;
  }
}

function is401(err: unknown): boolean {
  if (!err) return false;
  const msg = err instanceof Error ? err.message : String(err);
  return /\b401\b|unauthori[sz]ed/i.test(msg);
}

els.form.addEventListener('submit', (e) => {
  e.preventDefault();
  void load();
});

// --- Bootstrap ---------------------------------------------------------------

// If we landed on this page mid-OAuth (i.e. the URL carries ?code=), finish
// the exchange before the user presses Load so the token is warm. If there's
// no code we do nothing — authorization happens lazily on the first Load.
(async () => {
  const params = new URLSearchParams(window.location.search);
  if (params.has('code')) {
    try {
      setStatus('exchanging code…');
      accessToken = await getAccessToken({ baseUrl: BASE_URL });
      setStatus('ready — press Load');
    } catch (err) {
      console.error(err);
      setStatus(err instanceof Error ? err.message : 'oauth error', true);
    }
  } else {
    setStatus('ready — press Load');
  }
})();
