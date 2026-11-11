# chartgen auto-deploy

A systemd timer polls GitHub every 5 minutes. When `master` has a new head
SHA, its `CI` workflow succeeded and the image CI built for it is in the
registry, the script points the Coolify application on the same host at
`ghcr.io/5queezer/chartgen:<sha>` and POSTs Coolify's deploy API. The host
pulls the image and builds nothing. Nothing is exposed inbound and GitHub
holds no Coolify secrets. Rationale:
[ADR-0037](../../website/src/content/docs/decisions/0037-pull-based-deploy.md) and
[ADR-0060](../../website/src/content/docs/decisions/0060-image-built-in-ci.md).

Needs `curl` and `jq`. Run everything below as root on the Coolify host. All
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
COOLIFY_API_TOKEN=<Coolify API token that may change and deploy the application>
COOLIFY_APP_UUID=<uuid of the Docker Image application>
GITHUB_REPO=5queezer/chartgen
CHARTGEN_IMAGE=ghcr.io/5queezer/chartgen
```

`GITHUB_REPO` and `CHARTGEN_IMAGE` may be left out; these are their defaults.
The Coolify application must be of the kind **Docker Image**: the script sets
its image name and tag. On an application that builds from a Dockerfile the
tag would be ignored and a deploy would build on the host again.

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
> headless clients. If the application comes up on an empty volume, every
> client and every headless agent has to be authorized again, and scripts and
> alerts are gone. **Copy the data first, with the application stopped,** and
> check the copy before anything else.

1. **Find how the data is mounted.** In Coolify: the application, **Persistent
   Storage**. A chartgen application has one named volume there, mounted at
   `/root/.chartgen`. Coolify names it `<application uuid>-<name>`; the examples
   below write `<old volume>` for that full name. `docker volume ls` on the
   host shows it. If the page shows a host path instead of a volume, copy that
   directory instead.
2. **Stop the application** in Coolify, and pause the timer:
   `systemctl disable --now chartgen-autodeploy.timer`.
3. **Copy the volume and check the copy:**

   ```sh
   mkdir -p /var/backups/chartgen
   docker run --rm -v <old volume>:/from:ro -v /var/backups/chartgen:/to debian:bookworm-slim \
     cp -a /from/. /to/
   ls -la /var/backups/chartgen                      # chartgen.db is there, with its size
   sqlite3 /var/backups/chartgen/chartgen.db "PRAGMA integrity_check; SELECT count(*) FROM oauth_clients;"
   ```

   `integrity_check` must answer `ok`, and the count must be the number of
   clients you expect, not 0. Without `sqlite3` on the host, compare
   `sha256sum` of the file in the volume and in the copy.
4. **Make the image available:** the first green `CI` run on `master` after
   this change pushes it. Then, on GitHub: the repository's **Packages**,
   `chartgen`, **Package settings**, **Change visibility**, **Public**. Check
   from the host: `docker pull ghcr.io/5queezer/chartgen:<sha>`.
5. **Switch the application.** Two ways:

   **A. In place** (keeps the uuid, the volume, the environment, the domain and
   the port). Coolify's API and UI do not offer to change an application that
   builds from a Dockerfile into a Docker Image application. Changing the
   application's build pack inside Coolify itself is **not verified by this
   repository**; do it only after it worked on an application that does not
   matter. _Rehearsal: not yet recorded._ If it is done, set the image name to
   `ghcr.io/5queezer/chartgen` and the tag to the SHA from step 4 as well, and
   look at **Persistent Storage** again before starting: it must still show
   `<old volume>`.

   **B. A new application** (always possible). **New resource, Docker Image**,
   image `ghcr.io/5queezer/chartgen`, tag the SHA from step 4. Copy from the old
   application: every environment variable, the domain, the port (9315). Add
   a persistent storage with destination `/root/.chartgen`. **Do not start it
   yet:** it has a new, empty volume, `<new volume>`. Fill it from the copy:

   ```sh
   docker volume create <new volume>     # only if Coolify has not created it yet; use the name it shows
   docker run --rm -v /var/backups/chartgen:/from:ro -v <new volume>:/to debian:bookworm-slim \
     cp -a /from/. /to/
   docker run --rm -v <new volume>:/to:ro debian:bookworm-slim ls -la /to
   ```

   Leave the old application stopped, not deleted, until the new one is proven.
6. **Start it and check** (see [Is it healthy?](#is-it-healthy)): the page
   loads, and a client that was authorized before still is, with no new
   authorization. If it asks again, stop: the application is not on the data.
7. **Point the timer at it:** put the application's uuid into
   `/etc/chartgen-autodeploy.env` (a new one for B), install the new script
   (see [Install](#install)), and enable the timer.

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
after step 7, or do it at once with `deploy <sha>` below. Deleting
`last-sha` makes the next run record the head **without** deploying it.

## By hand

The script deploys any published image, or the one from before the last
deploy. Both leave the timer's state alone: a rollback is not undone at the
next tick, and the next commit on `master` deploys as usual.

```sh
run() { systemd-run --wait --pipe --quiet -p EnvironmentFile=/etc/chartgen-autodeploy.env \
  -p DynamicUser=yes -p StateDirectory=chartgen-autodeploy /usr/local/bin/chartgen-autodeploy.sh "$@"; }
run deploy <full commit sha>     # that image, now
run rollback                     # the image that ran before the last deploy
```

Every image stays in the registry under its SHA, so any earlier commit of
`master` that has one can be named. `rollback` needs a deploy by the new
script to have happened; before that, use `deploy <sha>`.

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
cat /var/lib/private/chartgen-autodeploy/last-sha       # master's head the timer last deployed
cat /var/lib/private/chartgen-autodeploy/previous-sha   # the one before it: what rollback deploys
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
| `Coolify did not take the image tag` | The API token may not change the application, or the uuid is wrong. |

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
