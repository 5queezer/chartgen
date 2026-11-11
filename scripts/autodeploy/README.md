# chartgen auto-deploy

A systemd timer polls GitHub every 5 minutes. When `master` has a new head
SHA, its `CI` workflow succeeded and the image CI built for it is in the
registry, the script points the Coolify application on the same host at
`ghcr.io/5queezer/chartgen:<sha>` and POSTs Coolify's deploy API. The host
pulls the image and builds nothing. Nothing is exposed inbound and GitHub
holds no Coolify secrets. Rationale:
[ADR-0037](../../website/src/content/docs/decisions/0037-pull-based-deploy.md) and
[ADR-0060](../../website/src/content/docs/decisions/0060-image-built-in-ci.md).

Needs `curl`, `jq` and `flock` (util-linux). Run everything below as root on the Coolify host. All
paths are outside `/home`, so the timer works after an unattended reboot.

## Install

```sh
install -m 755 scripts/autodeploy/chartgen-autodeploy.sh /usr/local/bin/
install -m 644 scripts/autodeploy/chartgen-autodeploy.service scripts/autodeploy/chartgen-autodeploy.timer /etc/systemd/system/
install -m 600 /dev/null /etc/chartgen-autodeploy.env
$EDITOR /etc/chartgen-autodeploy.env
systemctl daemon-reload
systemctl enable --now chartgen-autodeploy.timer
```

`/etc/chartgen-autodeploy.env`:

```
COOLIFY_URL=http://coolify.lan:8000
COOLIFY_API_TOKEN=<Coolify API token that may read, change and deploy the application>
COOLIFY_APP_UUID=<uuid of the Docker Image application>
GITHUB_REPO=5queezer/chartgen
CHARTGEN_IMAGE=ghcr.io/5queezer/chartgen
```

`GITHUB_REPO` and `CHARTGEN_IMAGE` may be left out; these are their defaults.
The Coolify application must be of the kind **Docker Image**: the script sets
its image name and tag. On an application that builds from a Dockerfile the
tag would be ignored and a deploy would build on the host again, so the
script reads the application first and refuses any other kind.

The first run records the current `master` SHA and does not deploy. To
deploy the current head right away, see [By hand](#by-hand).

## The image

CI's `image` job builds the repository's `Dockerfile` for `linux/arm64` on a
GitHub-hosted arm64 runner, after every other CI job passed for the same
commit, and pushes:

- `ghcr.io/5queezer/chartgen:<full commit sha>` for every push to `master`, and for
  any ref the workflow is run on by hand (**Actions, CI, Run workflow**);
- `ghcr.io/5queezer/chartgen:master`, moved only by a run on `master` whose commit is
  still `master`'s head when its build is done.

A run by hand uses the workflow file **of the ref it is run on**. A branch whose
`.github/workflows/ci.yml` has no `image` job, or no `workflow_dispatch` trigger,
gets no image: merge or rebase it onto a `master` that has them first.
Anyone with write access to the repository can publish an image of any ref
this way, under that ref's commit SHA.

The package is meant to be **public**. The repository is public and the image
holds what its `Dockerfile` builds from it and nothing else, so a public
package shows nothing that is not already public, and the host pulls without
a credential. A private package would need a GitHub token with
`read:packages` in Coolify's registry settings, and this script cannot check
for an image in a private package: it asks the registry as anyone would.

## Moving an existing application to the image, once

> **This is where the data can be lost.** `chartgen.db` holds the clients,
> users, sessions, approvals, stored scripts, alerts and the refresh tokens of
> headless clients. If the application comes up on an empty or a wrong
> database, every client and every headless agent has to be authorized
> again, and scripts and alerts are gone. The steps below are in this order
> on purpose: **nothing is stopped before the image is proven to pull, every
> copy goes into a place proven empty, and every copy is checked by what is
> in the database, not by the list of files.**

Run the commands as root on the host, **all in one shell:** they use the
variables `b` (the backup directory) and `v` (the new volume's directory) and
the function `check`, which a new shell does not have. Every command that
copies or moves is written with `"${b:?}"` and `"${v:?}"`, which stop with a
message when the variable is empty; an empty one would otherwise make `cp`
copy from or into `/`. If the shell is lost, define `check` again and set
`b=/var/backups/chartgen/<the directory made in step 3>` before going on.
`<old volume>`, `<new volume>` and `<sha>` stand for your values. The checks
need `sqlite3` on the host.

**The check, used three times below.** It opens the file read-only, so it
cannot create or change anything:

```sh
check() { test -s "$1" && sqlite3 -readonly "$1" "PRAGMA integrity_check; SELECT count(*) FROM oauth_clients;"; }
```

A good database answers with exactly two lines: `ok`, and the number of
clients, which is above 0 for a production store. **Anything else means
stop:** no output (the file is missing or empty), an `Error:` line, even if
`ok` follows it, or `0`.
Do not use plain `sqlite3 <file>` for this: on a missing file it creates an
empty one, prints `ok`, and leaves a file of the right name behind.

### 1. The image first, with production still running

1. Merge the change. The `CI` run of that commit on `master` builds and pushes
   the image; wait for its `image (linux/arm64)` job to be green. If it fails,
   production is untouched: fix that first.
2. On GitHub: the repository's **Packages**, `chartgen`, **Package settings**,
   **Change visibility**, **Public**.
3. On the host, prove the pull: `docker pull ghcr.io/5queezer/chartgen:<sha>`
   with the full SHA of that commit. Do not go on until this works.

### 2. Know the volume

In Coolify: the application, **Persistent Storage**. A chartgen application
has one named volume there, mounted at `/root/.chartgen`; Coolify names it
`<application uuid>-<name>`. That full name is `<old volume>`. Prove it exists:

```sh
docker volume inspect <old volume>      # an error here means the name is wrong: stop
```

This matters because `docker run -v <name>:...` does not fail on a wrong
name: it creates an empty volume of that name and copies nothing.

**Pin the old application to what it runs, now, while it runs.** It builds
from the repository and follows `master`, and `master` has moved since it was
last built (step 1 merged). If it is ever started again, as the way back
below, Coolify may build the new head on the host: the build this change
exists to avoid. So before stopping it, set the commit SHA field in the
application's source settings (it normally says `HEAD`) to the commit it is
running, which the newest finished deployment in **Deployments** names, and
save without deploying. _Not verified by this repository:_ the field's name
in your Coolify version, and whether a start then reuses the image built
before. Without the pin, treat a start of the old application as a build
on the host.

### 3. Stop, copy, check

From here until the application runs again, production is down.

1. Stop everything that deploys, in this order, **before** the application:

   ```sh
   systemctl disable --now chartgen-autodeploy.timer
   while systemctl is-active --quiet chartgen-autodeploy.service; do sleep 2; done
   systemctl is-active chartgen-autodeploy.service     # says: inactive
   ```

   Disabling the timer does not stop a run of the service that is already
   under way, and that run can still ask Coolify for a deploy. So wait for it
   to end. Then look at the application's **Deployments** in Coolify: nothing
   may be queued or in progress. A deployment that Coolify already holds can
   start the application again after you stopped it.
2. Stop the application in Coolify. The copy below runs only if no running
   container uses the volume: that is what catches a deployment that started
   the application again behind your back, at the moment of the copy.
3. Copy the volume into a **new** directory that only root can read:

   ```sh
   b=/var/backups/chartgen/$(date +%Y%m%d-%H%M%S)
   install -d -m 700 /var/backups/chartgen "${b:?}"
   if [ -z "${b-}" ]; then
     echo "NOT COPIED: b is not set."
   elif ! running=$(docker ps -q --filter volume=<old volume>) || [ -n "$running" ]; then
     echo "NOT COPIED: a container that uses <old volume> is running, or docker ps failed. Stop the application first."
   elif [ -n "$(ls -A "$b")" ]; then
     echo "NOT COPIED: $b is not empty. Set b to a new directory and start this block again."
   else
     docker run --rm -v <old volume>:/from:ro -v "${b:?}":/to debian:bookworm-slim cp -a /from/. /to/ &&
       chmod 700 "${b:?}" &&      # cp -a gave the directory the volume's own mode
       check "$b/chartgen.db"     # two lines: ok, and the number of clients. Write the number down.
   fi
   echo "$b"                  # write this down too: it is how to set b again in a new shell
   ```

   The block copies only if nothing runs on the volume and the directory is
   empty, and checks only what it has just copied. **A copy taken from a
   running server can check `ok` and still miss its last writes,** which is
   why it refuses while a container uses the volume.

   The source is mounted read-only and nothing in it is touched. `cp -a /from/.`
   takes `chartgen.db` and, if the application was not stopped cleanly, its
   `-wal` and `-shm` files, which belong to it.

**A copy is only good if the application has not run since it was taken.**
If you start the old application again for any reason, this copy is stale:
take a new one, into a new directory, before going on.

### 4. Switch the application

Two ways.

**A. In place** (keeps the uuid, the volume, the environment, the domain and
the port). Coolify's API and UI do not offer to change an application that
builds from a Dockerfile into a Docker Image application. Changing the
application's build pack inside Coolify itself is **not verified by this
repository**; do it only after it worked on an application that does not
matter. _Rehearsal: not yet recorded._ If it is done, set the image name to
`ghcr.io/5queezer/chartgen` and the tag to `<sha>` as well, and look at
**Persistent Storage** again before starting: it must still show
`<old volume>`. Then check the data where it is:

```sh
check "$(docker volume inspect -f '{{.Mountpoint}}' <old volume>)/chartgen.db"   # ok, and the same number
```

**B. A new application** (always possible). **New resource, Docker Image**,
image `ghcr.io/5queezer/chartgen`, tag `<sha>`. Copy from the old application:
every environment variable, the domain, the port (9315). Add a persistent
storage with destination `/root/.chartgen`. **Do not start it.** Its volume,
`<new volume>`, is the name Coolify shows under **Persistent Storage**.

Fill the volume, and only if it is empty:

```sh
docker volume inspect <new volume> || docker volume create <new volume>   # create only if Coolify has not yet
v=$(docker volume inspect -f '{{.Mountpoint}}' <new volume>)
echo "new: ${v:?}"; echo "old: $(docker volume inspect -f '{{.Mountpoint}}' <old volume>)"
```

**Read the two lines. They must be two different directories.** If they are
the same, `<new volume>` was typed as the old volume's name: stop, nothing
below may be run on it.

```sh
if [ -z "${b-}" ] || [ -z "${v-}" ]; then
  echo "NOT COPIED: b or v is not set."
elif ! running=$(docker ps -q --filter volume=<new volume>) || [ -n "$running" ]; then
  echo "NOT COPIED: a container that uses <new volume> is running, or docker ps failed. Stop the new application first."
elif [ -n "$(ls -A "$v")" ]; then
  echo "NOT COPIED: $v is not empty, see below."; ls -A "$v"
else
  cp -a "${b:?}"/. "${v:?}"/ &&
    check "$v/chartgen.db"     # ok, and the same number as in step 3
fi
```

The copy runs only into an empty volume that no running container uses, and
the check runs only on what was just copied: a `NOT COPIED` is never followed
by an `ok`. If it says the volume is not empty, **look before touching it:**

```sh
check "$v/chartgen.db"
```

- `ok` and `0`: the new application has already run once,
  and this is the database of a server that started empty, with its `-wal` and `-shm` files
  beside it. Go on below.
- `ok` and the number from step 3, or any number above 0: **stop. This is
  not an empty server's volume; most likely it is the old one.** Move
  nothing.
- Anything else: stop.

Copying over an empty server's files would put the good `chartgen.db` next
to that server's `-wal`. SQLite applies that log to the good file at the
next start, and the server comes up with no clients while `integrity_check`
still says `ok` and the file list looks right. (Tried: 3 clients copied in, 0
seen by the server.) So stop the new application and move its three files
out of the volume, into a new directory beside the backup. Nothing is
deleted:

```sh
aside="${b:?}-moved-out-of-new-volume"; install -d -m 700 "$aside"
for f in chartgen.db chartgen.db-wal chartgen.db-shm; do
  [ ! -e "${v:?}/$f" ] || mv "${v:?}/$f" "$aside/"
done
ls -A "${v:?}"             # prints nothing now; then the copy block above again
```

Whether Coolify takes a volume that was created by hand under the name it
shows is **not verified by this repository**; rehearse it on staging. If it
does not, start the application once, stop it, and empty and fill the
volume as above.

Leave the old application stopped, not deleted, and the backup directory in
place, until the new one is proven.

### 5. Start and check

Start the application, then (see [Is it healthy?](#is-it-healthy)):

- Coolify's deployment log shows a pull of `ghcr.io/5queezer/chartgen:<sha>` and
  no build.
- The container's log has the line `OAuth client store: ... (N clients
  restored)`, and `N` is the number from step 3.
- A client that was authorized before still is, with no new authorization.

If the number is 0 or a client is asked to authorize again: **stop the
application.** It is not on the data. The backup in `"$b"` and the old
volume still have it.

### 6. Point the timer at it

Install the new script (see [Install](#install)), put the application's
uuid into `/etc/chartgen-autodeploy.env` (a new one for B), and enable the
timer. The script reads the application before every deploy and refuses
one that is not a Docker Image application, so the old uuid left in that
file by mistake deploys nothing and says so in the log.

Then tell the script what runs, so that its first rollback has somewhere to
go: `run running <sha>` with the commit you started in step 4 (`run` is
defined under [By hand](#by-hand)).

### If it goes wrong before the new application is proven

The data is in two places whatever happened: the backup directory, and
`<old volume>`, which no step writes to on route B.

- **Route B:** stop the new application and start the old one; it is still
  on `<old volume>`. **Only if it was pinned in step 2:** unpinned, a start
  may build `master`'s head on the host. If it was not pinned, pin it first,
  to the commit of its newest finished deployment. Once the old application
  has run again, the backup is stale; see step 3.
- **Route A:** there is no old application to start; the application itself
  was changed. Either undo the change in Coolify and start it, with the same
  caution about a build on the host, or leave it stopped and go on with
  route B, filling a new application's volume from the backup. Check
  `<old volume>` with `check` first to see what it holds.

### What the timer does the first time

The state file from before this change still holds the last SHA the old
script deployed. The new script reads it as it always did:

- `master` is still at that SHA: nothing happens until the next commit.
- `master` has moved on: the next tick deploys `master`'s head through the new
  path, as soon as its CI is green and its image is published. Commits from
  before this change have no image, so the script waits and says so. They
  never get one; the first commit that can be deployed is the merge of this
  change.

So to get the merged code out the first time, either let the timer do it
after step 6, or do it at once with `deploy <sha>` below. Deleting
`last-sha` makes the next run record the head **without** deploying it.

## By hand

```sh
run() { systemd-run --wait --pipe --quiet -p EnvironmentFile=/etc/chartgen-autodeploy.env \
  -p DynamicUser=yes -p StateDirectory=chartgen-autodeploy /usr/local/bin/chartgen-autodeploy.sh "$@"; }
run deploy <full commit sha>     # that image, now
run rollback                     # the image the script triggered before its last one
run running <full commit sha>    # tell the script what runs now; deploys nothing
```

`deploy` takes a full commit SHA, 40 hex characters, and nothing else: a
name like `master` is a tag that moves. Every image stays in the registry
under its SHA, so any earlier commit of `master` that has one can be named.

**What the timer does after a command by hand.** A `deploy` or `rollback` is
your word on what runs, so the timer does not put `master`'s head of that
moment out over it: it marks that head as dealt with, without deploying it.
That head may be the very commit you rolled back. **The next commit on
`master` deploys as usual.** To put the head out after all, name it:
`run deploy <sha>`. If GitHub cannot be reached when you run the command,
the first tick that reaches it does the same and says so in the log.

**What `rollback` knows.** The script records the image it triggered last,
by the timer or by hand, and the one it had triggered before that;
`rollback` deploys the latter, and prints its SHA before it does. So after
"the timer deployed A, then B, and B is bad", `rollback` deploys A, and when
the timer later deploys C, the next `rollback` goes to A again.

It knows only what it triggered itself, and **"triggered" means Coolify
accepted the request, not that the image ever ran.** If B was triggered and
failed in Coolify, and C came after it, `rollback` names B. So read the SHA
it prints; if in doubt, look up in Coolify's **Deployments** which commit
last ran well and use `deploy <sha>`.

- **A second `rollback` in a row goes forward again:** it deploys what the
  first one replaced, the image you just rolled back from. To go further
  back, name the commit with `deploy <sha>`.
- **Before the script has triggered anything,** it knows no earlier image.
  The timer's first run only notes where `master` is; that is not a
  deployment, and `rollback` does not use it. `rollback` then deploys
  nothing and says so. Tell the script what runs when it takes over, so
  that the first rollback has somewhere to go: `run running <sha>` with the
  commit of the image the application runs.
- After a deploy made in Coolify's UI the script's record is out of date:
  `run running <sha>`, or name the SHA with `deploy`.

Only one run works at a time, the timer's or yours. A command by hand
waits up to two minutes for a tick under way to finish; if it cannot get
its turn it fails with `nothing was deployed`, and you run it again. A tick
that finds a command under way does nothing.

## Is it healthy?

The script logs `triggered Coolify deploy` when Coolify accepted the request.
**That is not "healthy":** it does not wait for the deployment and does not
look at the application. To check:

- Coolify, the application, **Deployments**: the newest one is finished, and
  its log shows the pull of `ghcr.io/5queezer/chartgen:<sha>` and no build.
- `curl -s https://<your domain>/` answers with the health JSON.
- `docker ps --format '{{.Image}} {{.Status}}'` on the host shows the SHA.

## Pause / resume

```sh
systemctl disable --now chartgen-autodeploy.timer   # pause
systemctl enable --now chartgen-autodeploy.timer    # resume (deploys the newest green master, if not yet deployed)
```

## Logs and state

```sh
journalctl -u chartgen-autodeploy.service -n 50
systemctl list-timers chartgen-autodeploy.timer
cat /var/lib/private/chartgen-autodeploy/last-sha       # master's head the timer last dealt with (deployed, or left alone)
cat /var/lib/private/chartgen-autodeploy/deployed-sha   # what this script triggered last, or was told runs
cat /var/lib/private/chartgen-autodeploy/previous-sha   # what it had triggered before that: what rollback deploys
```

Delete `last-sha` to make the next run re-baseline (it records the head
without deploying).

What the log says when nothing is deployed:

| Line | Meaning |
|------|---------|
| `CI for <sha> not finished` | Wait. On `master` the run now includes the image build. |
| `CI for <sha> concluded 'failure'` | Fixed by a new commit, or by re-running the failed jobs if it was a flaky one. |
| `CI for <sha> concluded 'cancelled'` | See below. |
| `not in the registry yet` | CI is green and the image is not there. Seconds after a run this is normal; if it stays, the commit is from before the image job, or the job did not run. |
| `is the package public?` | The registry refused. Set the package to public. |
| `registry lookup ... answered 429` | Rate limit or network; the next tick tries again. |
| `another run is under way` | A command by hand, or a slow tick, holds the lock. The next tick comes anyway. |
| `not deploying <sha>: an image was deployed by hand` | A command by hand came first; this head is left alone and the next commit deploys. |
| `not a Docker Image application` | `COOLIFY_APP_UUID` names an application that builds on the host, usually the old one. Nothing was changed or deployed. |
| `could not read the Coolify application` | The uuid is wrong, the token may not read applications, or Coolify is down. |
| `Coolify did not take the image tag` | The API token may not change the application. |

## Cancelled or timed-out CI

If the `CI` run for a new head is `cancelled` or `timed_out` (for example a
stalled mirror), the script logs it and does not deploy. Nothing else will
deploy that SHA, so re-run CI (`gh run rerun <run-id>` or the Actions UI);
the next tick picks up the green run. A CI run that failed on tests is
fixed by a new commit instead.

## Uninstall

```sh
systemctl disable --now chartgen-autodeploy.timer
rm /etc/systemd/system/chartgen-autodeploy.{service,timer} /usr/local/bin/chartgen-autodeploy.sh /etc/chartgen-autodeploy.env
rm -r /var/lib/private/chartgen-autodeploy
systemctl daemon-reload
```

## Test

`scripts/autodeploy/test.sh` runs the script against a stub `curl` that plays
GitHub, the registry and Coolify; no network.
