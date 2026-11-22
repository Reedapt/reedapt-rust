# Changelog

## 0.2.0

First release, matching the Node.js and Python libraries at their 0.2.0.

Uploads in parts (from a path, bytes or any async reader) and from links, dubs (create, retrieve, list, `list_all` as a `Stream`, cancel, files, transcript, subtitles, quality, `wait_until_done`), languages, usage and the monthly usage report, webhook endpoints (including `resend_delivery`), webhook signature checks, retries with backoff honouring `Retry-After`, and idempotency keys.

`instructions` on dub creation, the `dub.checked` event, and the early-access requests, runs and lines calls.

An optional `blocking` feature adds `reedapt::blocking::Client`.
