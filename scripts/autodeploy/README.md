# chartgen auto-deploy

A systemd timer polls GitHub every 5 minutes. When `master` has a new head
SHA and its `CI` workflow succeeded, the script POSTs Coolify's deploy API on
the same host. Nothing is exposed inbound and GitHub holds no Coolify secrets.
Rationale: [ADR-0037](../../website/src/content/docs/decisions/0037-pull-based-deploy.md).

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
COOLIFY_URL=http://192.168.1.242:8000
COOLIFY_API_TOKEN=<Coolify API token with deploy permission>
COOLIFY_APP_UUID=<application uuid>
GITHUB_REPO=5queezer/chartgen
```

The first run records the current `master` SHA and does not deploy. To
deploy the current head right away, trigger it from Coolify.

## Pause / resume

```sh
systemctl disable --now chartgen-autodeploy.timer   # pause
systemctl enable --now chartgen-autodeploy.timer    # resume (deploys the newest green master, if not yet deployed)
```

## Logs and state

```sh
journalctl -u chartgen-autodeploy.service -n 50
systemctl list-timers chartgen-autodeploy.timer
cat /var/lib/private/chartgen-autodeploy/last-sha   # last SHA a deploy was triggered for
```

Delete `last-sha` to make the next run re-baseline (it records the head
without deploying).

## Uninstall

```sh
systemctl disable --now chartgen-autodeploy.timer
rm /etc/systemd/system/chartgen-autodeploy.{service,timer} /usr/local/bin/chartgen-autodeploy.sh /etc/chartgen-autodeploy.env
rm -r /var/lib/private/chartgen-autodeploy
systemctl daemon-reload
```

## Test

`scripts/autodeploy/test.sh` runs the script against a stub `curl`; no network.
