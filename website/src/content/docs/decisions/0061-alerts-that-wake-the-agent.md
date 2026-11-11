---
title: "0061 — An alert can wake the owner's agent"
description: "A server alert can be delivered to one receiver the operator configures for one owner, signed the Standard Webhooks way and carrying the setup it is about. No request names the receiver's URL, and its secret never passes through a tool call."
sidebar:
  order: 61
---

**Status:** Accepted
**Date:** 2026-11-11
**Amended by:** [ADR-0067](/chartgen/decisions/0067-indicator-alerts-on-the-server/) (an indicator alert's wake-up carries the indicator; `level` is the one stored with the fire)
**Amends:** [ADR-0042](/chartgen/decisions/0042-server-side-alerts/) (deliveries also carry the
Standard Webhooks headers; one receiver inside, which only the operator can name; wake-ups have a
queue, a pace and slots of their own; a wake-up is tried again for up to an hour),
[ADR-0059](/chartgen/decisions/0059-setups-paper-scored/) (each status the server gives a
setup is also an event of its owner's log)

## Context and Problem Statement

An agent that trades on paper works in a loop: something wakes it, it looks, it stores a
setup ([ADR-0059](/chartgen/decisions/0059-setups-paper-scored/)) or sets an alert, and the
alert wakes it again. The agent keeps its access by itself
([ADR-0058](/chartgen/decisions/0058-refresh-tokens-for-headless-clients/)). What was missing
is the wake-up.

Server alerts ([ADR-0042](/chartgen/decisions/0042-server-side-alerts/)) already call a
webhook, but not one an agent on the same host can receive:

- The signature is this server's own format. A receiver built for a common convention
  refuses it.
- A webhook only goes to a public address, over https, on port 443. The agent's receiver is
  a container next to the server, with no public name and no certificate.
- The secret is made per alert and shown once to whoever created the alert. A receiver has
  one secret per route, and the creator here is the agent, so the secret would pass through
  the model's context.
- The body says which alert fired. The agent wants to know what the alert was about.

How does an alert reach the owner's own agent without opening a way from a tool call to the
inside of the network?

## Decision Drivers

- No URL that a request supplies may ever reach a non-public address. This rule protects a
  server on a home network from every client that can register.
- The receiver's secret is the operator's. It does not pass through a tool call.
- What a wake-up carries, the owner's thesis, goes to the owner's receiver and nowhere else,
  and never to a log.
- A receiver that hangs or is away must not cost any other owner anything.
- An alert changes nothing on a setup: the paper score stays the server's alone.
- Existing receivers keep working, byte for byte.

## Considered Options

For reaching the receiver:

1. **A public name for it,** through the tunnel, and the existing rules.
2. **A list of inside origins,** set by the operator, that a `webhook_url` may then name.
3. **One receiver that the operator configures whole,** URL, owner and secret, which an alert
   asks for with a flag and no request can name.

For the signature:

- a. Add the Standard Webhooks headers beside the existing ones.
- b. Replace the existing ones.
- c. Add the generic format of one receiver.

## Decision Outcome

Chosen: **3** and **a**.

### The receiver

Three settings, read once at startup, all of them or none:

| Setting | What it is |
|---|---|
| `CHARTGEN_AGENT_WEBHOOK_URL` | The receiver's whole URL, with its path. |
| `CHARTGEN_AGENT_WEBHOOK_OWNER` | The one owner whose alerts may ask for it: an account with sign-in on, a client id with it off. |
| `CHARTGEN_AGENT_WEBHOOK_SECRET` | The signing key, the same string the receiver has: `whsec_<base64>` or any string of at least 24 characters. |

An alert asks for it with `notify_agent: true`. There is no URL in the call, so the rule for
`webhook_url` is untouched: public addresses only, as before.

The rule for the receiver itself:

- **`http` or `https`, any port but this server's own.** A user or password in the URL is
  refused.
- **Its address must be inside, by a list of what is allowed:** loopback, the private IPv4
  ranges and unique-local IPv6. Not "everything that is not public": link-local, where every
  cloud keeps its metadata service, the unspecified address, multicast, broadcast and the
  mapped forms are refused. A name with no dot is taken, as a container has one.
- **Resolved and pinned at every attempt,** as every webhook is. If a name answers with one
  address outside the list among good ones, the delivery is refused.
- **No redirects, and never through `CHARTGEN_WEBHOOK_PROXY`.** The proxy is there to hide this
  server from strangers.
- **The owner must be named in `CHARTGEN_SERVER_ALERT_CLIENTS`.** `*` allows everyone and names
  nobody.
- Anyone else who sets `notify_agent` gets the answer every webhook refusal gives. It does
  not tell that a receiver exists. The operator's log gets the refused owner's id and
  nothing of the request.
- A setting that is wrong leaves server alerts off, with an `ERROR` line at startup that
  names the setting and not its value. That is what a wrong webhook setting already did.
  Setups go on being scored.

**Plain `http` is allowed here and only here.** The receiver is on the host's own network and
has no certificate. The signature keeps a body from being forged or changed; it does not
keep it from being read. What keeps it from being read is that it never leaves the host:
whoever can read that network's traffic already has the host. Put the two containers on a
network that nothing else shares.

### The signature

Every delivery, of any alert, now also carries `webhook-id`, `webhook-timestamp` and, when
signed, `webhook-signature`: `v1,` and the base64 HMAC-SHA256 of `<id>.<timestamp>.<body>`.
The id is the event's and the same on every attempt, so a receiver can drop a repeat. The
timestamp is the attempt's, in seconds. An unsigned delivery has no signature header at all.
The `X-Chartgen-*` headers and the body of an existing alert are unchanged.

### The body, and the setup

An alert with `notify_agent` may name one of the owner's open setups, `setup_id`. The
wake-up then carries the setup as it stands when the delivery is sent: id, status, symbol,
levels, fill, exit, result, and the thesis. Not its drawings or its log, which `get_setup`
answers.

**The order of the fields is fixed by how the body is built:** id, type, time, the alert, its
note, then the setup with its status and levels first and the thesis last. A receiver that
puts only the beginning of a body in front of the agent still shows where the setup stands.
A body is at most 8 KiB; a thesis that would exceed it is cut and marked.

The body is made when it is sent. What waits in the store for its turn is the event alone,
so a thesis is never copied into the event table. **The event says what happened; the setup
in the body is the setup as it is when the body is sent.** A wake-up that gets through late
can therefore say "triggered" about a setup that already shows `won`.

**The link is a reference and nothing more.** An alert changes nothing on a setup, linked or
not, and no tool takes a status as input. Deleting the alert leaves the setup alone.

In this version a setup can be named only together with `notify_agent`, and an alert takes
`webhook_url` or `notify_agent`, not both: an alert has one webhook outcome to show.

### Limits

Wake-ups have their own: **50 waiting per owner, 30 a minute, 2 at once.** They are not counted in the
budgets of ADR-0042, and nothing there is counted in theirs. A receiver that takes
connections and never answers holds those 2 slots and nobody else's. Other owners'
deliveries cannot crowd a wake-up out. The alert's own limits (20 per owner, the watched
pairs, changes per minute) apply as to any alert.

### The event log

Everything that could wake an owner's agent is first written to a log in the store, one row
per event, and stays there whether or not a wake-up was sent or got through. There are two
kinds:

- `alert.fired`: an alert with `notify_agent` fired. Written in the transaction that stores
  the fire.
- `setup.status`: the server gave a setup a status (`triggered`, `won`, `invalidated`,
  `expired`), or its owner cancelled it. Written in the transaction that stores the setup's
  own log entry, with **the time of the bar** that decided it, not the time of the write.

An event holds what happened, as a few numbers and words, and the ids of the alert and the
setup. It never holds a thesis or a note.

Events are numbered per owner, from 1, and a number is never used again, also after the
events before it are gone. The server keeps **the newest 1000 events of an owner, for 7
days**. `list_events` reads them back after a cursor, and says `truncated` when events
after the caller's cursor are no longer kept.

A replay server writes no events and refuses `list_events`: its answers must not depend on
anything a run leaves behind.

### Which status changes wake the agent

**A status the server gives a setup wakes the agent without any flag on the setup,** if the
setup's decision is `take` and its owner is the one the receiver is configured for. The body
then has `change` (`from`, `to`, `gap`, and `result_r` where there is one) in the place of
`alert`, and the setup after it.

**A cancellation and every change of a `skip` setup are logged and wake nobody.** A
cancellation is the agent's own act, and nobody waits on a setup that was not taken. The
same holds for the setups of every other owner: logged for them, delivered to no one.

### Attempts over a longer time

The log is also the queue. A wake-up is tried three times within about fifteen seconds, as
any webhook is. If it got no answer, a 5xx or a 429, it is tried once more after 1, 5, 15
and 30 minutes, each counted from the attempt before, **while the event is younger than one
hour**. Then it is given up and marked `failed`. Any other answer is the receiver's refusal
and is not tried again. The hour is the time a Standard Webhooks receiver remembers an id
for, so a second arrival of one event is still recognised as the same.

The times are in the store with the event. A restart continues the schedule where it was,
and does not start it again. An alert shows the outcome of its newest attempt.

What is not delivered within the hour is not lost: it is in the log, and an agent that
was away reads it with `list_events`.

### Consequences

- Good: no way from a tool call to a non-public address was added. The one target inside
  is in the operator's configuration.
- Good: the secret is never returned, stored in a record or passed through MCP.
- Good: a receiver built for Standard Webhooks verifies a delivery without code for this
  server, and existing receivers see no change.
- Good: a thesis goes to the owner's receiver only, and is not in the event table.
- Good: a receiver that is away for a deploy gets its wake-ups when it is back, within the
  hour, and the log answers for anything older.
- Bad: one receiver and one owner per server. A second agent needs a public receiver and
  `webhook_url`, without the setup in the body.
- Bad: the operator has to learn the owner's id and type it. With sign-in off it is a
  client id, and a client that registers again is a new owner: the server then logs the
  refused id, and the setting has to follow it. Sign-in on, where the owner is the account,
  is the better fit.
- Bad: changing any of the three settings takes a restart.
- Bad: a wake-up can arrive up to an hour after what it reports, and more than once if an
  answer was lost on the way. The receiver has to drop a repeated `webhook-id`.
- Bad: there is no switch on a setup for its status wake-ups. Every `take` setup of the
  agent's owner wakes it.
- Bad: **a build from before the log does not write to it.** On a store this build has
  migrated it runs as before, since it ignores tables it does not know: alerts fire and
  setups are scored, and none of that is in the log. After a roll forward the log has a
  hole for that time, which no `truncated` reports, and a wake-up that waited through it
  is tried again only if it is still younger than an hour.
- Bad: **a build from before this decision that writes a linked alert drops the link.** It
  reads the record, ignores the two fields it does not know and runs the alert as one with
  no webhook; when it stores the alert again, after a fire or an update, the fields are
  gone. After a rollback and a roll forward, such alerts have to be given `notify_agent`
  again.
- Neutral: the stored alert record gains `notify_agent` and `setup_id`, written only when
  set. The log is two new tables, `events` and `event_counters`, added by the tenth
  migration; no existing table changes.

## Pros and Cons of the Options

### 1. A public name

- Good: no new rule.
- Bad: a new public surface, for a body that carries the owner's reasoning.
- Bad: it does not solve the secret, which is still per alert and shown to the agent.

### 2. A list of inside origins for `webhook_url`

- Good: several receivers.
- Bad: a URL from a request then reaches the inside. Every check of the list becomes part
  of what keeps strangers out of the network.
- Bad: the secret problem stays.

### 3. One receiver in the configuration

- Good and bad: as under Consequences.

### The signature options

- a. Good: nothing breaks, and the convention is one many receivers know.
- b. Bad: a wire break for every receiver that verifies today.
- c. Bad: one receiver's private format.

## Validation

Unit tests check the signature against the convention's own published example and with a
receiver written as the agent's is; the address list in both directions; that a client's
`webhook_url` cannot be the receiver's; that mixed answers of a name are refused; the
settings; the fixed order of the body, with a thesis of full length rendered as the
receiver renders it; the refusals; and that a hung receiver holds no other owner's
delivery. Two tests run the real binary: a wake-up on a receiver that answers, one that
keeps failing and one that is absent, with nothing of the body, the path or the secret in
what the server prints; and a wrong setting, which turns alerts off, says so, and leaves
setups scored.

For the log: that an event and what it reports are stored together or not at all; the
numbering across pruning; the schedule, attempt by attempt, to the give-up; that a restart
continues it; which answers are tried again; the hour; the cap of waiting events; status
events at the bar's time; that a cancellation, a skip and another owner's setup wake
nobody; that a replay writes none; and `list_events` with its cursor, `more`, `truncated`
and bounds. The real binary is run against a receiver that keeps failing until the attempt
one minute later arrives, with nothing of the body in what the server prints. Two older
builds were run on a file this build had migrated: both worked as before and left the log
as it was.

Not validated when this was written: the agent's real receiver, sign-in on, and the give-up
after an hour in the real binary.
