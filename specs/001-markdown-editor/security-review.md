# Security Review (T135)

Audit of Emend against Constitution **Principle II** (least privilege / sandbox) and
**Principle III** (privacy by default — zero outbound network unless AI is configured
*and* invoked; secrets never persisted or logged). Scope per the task: every outbound
call, API-key handling, and the atomic-write path. Citations are `file:line` at the
time of writing (`main` + this Polish branch).

## 1. Outbound network surface

**Finding: there is exactly one outbound network path in the entire codebase, and it
is the BYOM AI client.** A full sweep found:

- **Rust core (`emend-core`)** — no HTTP client at all. Verified structurally: the
  isolation guardrail (`cargo tree -p emend-core -i reqwest/tokio/uniffi` finds
  nothing) keeps all networking out of the core. The core's AI module
  (`crates/emend-core/src/ai.rs`) is *pure*: SSE parsing, the input-size guard, the
  request/auth-header *builders*, and the redacting key type — no sockets.
- **Rust FFI (`emend-ffi`)** — the only client is `reqwest` in
  `crates/emend-ffi/src/ai.rs`: `client.post(&url)…send()` (line 263) and the
  `bytes_stream()` response (line 313), plus a probe `post` for *Test Connection*
  (line 433). The client is built once (`build_client`, line 465) with
  `redirect::Policy::none()` (line 467) — a server cannot 30x-redirect the
  authenticated request (with its `Authorization` header) to an attacker origin.
  `reqwest` is `default-features=false, features=["stream","native-tls"]` (macOS
  Security.framework TLS; no bundled roots).
- **Swift app** — **no `URLSession`/`URLRequest`/`dataTask` anywhere.** All
  networking is delegated to the Rust client above; Swift only ever holds the
  transient key string to hand across the FFI.

### Zero-network gating (SC-008 / FR-035) — verified

The request is gated **before any socket** in `summarize_document`
(`crates/emend-ffi/src/ai.rs:157`) and `test_ai_config` (line 407):

- A blank/absent key → `FfiError::AiNotConfigured` and **no request** (`key.is_blank()`
  check, line 172 / 410) — so an un-configured app makes zero outbound connections.
- Oversized input → `FfiError::AiOversizedInput` via `check_input_size`
  (`emend-core/src/ai.rs`), again before the socket (line 186).
- Only after both pass is the request spawned on the tokio runtime (line 212).

This means the network is reachable **only** when the user has configured a key
*and* invoked summarize/test — exactly the SC-008 contract. There is no telemetry,
no update check, no analytics, no background fetch.

### Sandbox entitlement — FINDING (fixed in this branch)

`app/Emend/Emend/Emend.entitlements` enabled the App Sandbox and user-selected file
access + app-scoped bookmarks, but **lacked `com.apple.security.network.client`.**
Under the App Sandbox, outgoing connections are denied at the socket layer without
this entitlement — and because the AI client is `reqwest` over raw tokio TCP (not
`URLSession`/CFNetwork), the connection would simply fail in a sandboxed build, so the
shipped US6 AI feature would be non-functional.

**Resolution (this branch):** added `com.apple.security.network.client` with a comment
documenting that privacy is enforced *in code* (the SC-008 gating above), not by
withholding the entitlement. This is the minimal least-privilege addition for an app
with an AI feature:

- No *server* entitlement (`network.server`) — the app never listens.
- The preview WebView still cannot reach the network even with this entitlement
  granted (see §4) — its CSP is `connect-src 'none'` and remote navigations are
  cancelled.
- App Transport Security does **not** apply to the `reqwest` path (ATS gates
  CFNetwork/`URLSession` only), so a user pointing the BYOM endpoint at a local model
  over `http://localhost:…` works without an ATS cleartext exception — and we add no
  blanket `NSAllowsArbitraryLoads`, keeping the default-deny posture for any future
  CFNetwork use.

## 2. API-key handling (NFR-006) — verified clean

The BYOM key's custody is Swift-side, transient across the FFI, and redacted in Rust:

- **Storage:** Keychain only (`app/Emend/Emend/Platform/KeychainStore.swift`) as a
  generic password, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` (device-local,
  never synced to iCloud; line 32). Save is delete-then-add (clean replace). No copy
  is written to `UserDefaults`, a file, or a log.
- **Transit:** read immediately before a request and passed to Rust as a plain
  `String` parameter (`summarize_document(…, api_key: String)`); it is wrapped at the
  boundary into the redacting `ApiKey` newtype.
- **Redaction:** `emend_core::ai::ApiKey` (`crates/emend-core/src/ai.rs:60`) renders
  `***` for **both** `Debug` and `Display` (line 92+); the real bytes are reachable
  only via the explicit `expose()` accessor (line 78). `build_auth_header` (line 341)
  is the **sole** caller of `expose()`, forming `Bearer <key>` for the `Authorization`
  header and nowhere else.
- **No leakage on the error path:** a non-2xx response is mapped to `FfiError::AiHttp`
  with a **key-free** canonical-reason detail (`crates/emend-ffi/src/ai.rs:280-291`);
  transport errors go through `transport_error` (line 478) which carries no header
  data. No `println!`/`eprintln!`/`log`/`tracing`/`os_log`/`NSLog`/`print` references
  exist in the AI client or KeychainStore (verified by grep) — the key is never on a
  logging surface.

**Residual (accepted):** the key necessarily exists as plaintext in process memory
for the duration of a request (unavoidable to set the header). It is not zeroized on
drop. For a local, single-user desktop app with no key persistence outside the
Keychain this is an accepted, low-risk residual; zeroizing (`zeroize` crate) is a
possible hardening if the threat model later includes core-dump capture.

## 3. Atomic-write durability path (FR-009a) — verified

`emend_core::fs::write_atomic` (`crates/emend-core/src/fs.rs:93`) implements the
durable sequence and is the single write gateway:

1. Create a `NamedTempFile` in the **same directory** as the target (so the final
   `rename(2)` is atomic on one filesystem).
2. Write, `flush`, then `sync_all` the temp file. On Apple targets Rust std's
   `sync_all` is `fcntl(fd, F_FULLFSYNC)` (documented at `fs.rs:21-48` with the
   rust-lang rationale), giving true power-loss durability without a manual `fcntl`.
3. `persist` (atomic `rename`) over the target.
4. `sync_all` the **containing directory** so the rename itself is durable.

Properties relevant to security/integrity: a concurrent reader (or a crash at any
point before the rename) sees either the old complete file or the new complete file,
never a torn write (`fs.rs:84`). Failure of any step returns `EmendError` rather than
leaving a partial file. Autosave is debounced (≈1.5 s idle / 5 s hard cap) precisely
because `F_FULLFSYNC` is expensive (`fs.rs:50`) — durability without per-keystroke
disk thrash. Reads go through `read_tolerant`, and `Document::open` enforces the
5 MB `MAX_NOTE_BYTES` cap by stat-ing **before** allocating, so a hostile/huge file
cannot OOM the process (FR-027a; `crates/emend-core/src/document.rs:126-135`).

## 4. Preview WebView isolation (SC-008 / FR-035) — verified

The preview is offline by construction (`app/Emend/Emend/Preview/PreviewWebView.swift`
and `PDFExport.swift`):

- **CSP** (`Resources/preview/template.html:17`): `default-src 'none'` with only
  `'self'` for locally-bundled script/style/font/img, **`connect-src 'none'`**,
  `base-uri 'none'`, `form-action 'none'`. `img-src` allows `self data: file:` only.
- **Ephemeral store:** `config.websiteDataStore = .nonPersistent()`
  (`PreviewWebView.swift:24`, `PDFExport.swift:134`) — no cache/cookies persisted.
- **Navigation delegate** (`PreviewWebView.swift:107`): allows only `file:`/`about:`
  URLs; any remote navigation is `.cancel`-led, and a user-clicked link is handed to
  the system browser via `NSWorkspace` instead of loading in-app. Content is loaded
  with `loadFileURL(…, allowingReadAccessTo:)` scoped to the bundle preview dir.

Net: even though §1 now grants `network.client` for the AI client, the preview pane
has no path to the network — defense in depth.

## 5. FFI robustness (NFR-003) — verified

`emend-core` denies `unwrap_used`/`expect_used`/`panic` outside `#[cfg(test)]`;
fallible boundary calls return `Result<_, EmendError>` and UniFFI's `catch_unwind`
contains any residual panic as a catchable Swift `FfiError`. The AI streaming path
additionally wraps client construction in panic containment and treats an unexpected
panic as a contained terminal error (`crates/emend-ffi/src/ai.rs:244-250`). This is a
robustness property with a security dimension: a malformed server response cannot
crash the host process.

## Conclusion

The privacy and least-privilege model is sound and, with the
`com.apple.security.network.client` entitlement added in this branch, also
**functional**: exactly one code-gated outbound path (the BYOM AI client), a
Keychain-only/redacted/never-logged key, a durable atomic-write path, and a
network-isolated preview. One real finding (missing network entitlement) was fixed;
the remaining items are accepted low-risk residuals (in-memory key lifetime) and
tracked perf follow-ups (see `perf-report.md`), none of which block release.

| # | Finding | Severity | Status |
|---|---------|----------|--------|
| 1 | Sandbox missing `com.apple.security.network.client` → AI feature non-functional in sandboxed build | Medium (functional; privacy unaffected) | **Fixed** (entitlement added, privacy code-gated) |
| 2 | API key held as plaintext in process memory for the request lifetime, not zeroized | Low | Accepted (single-user desktop; `zeroize` is optional future hardening) |
| 3 | No ATS cleartext exception for `http://localhost` BYOM endpoints | Informational | No action — `reqwest` bypasses ATS; default-deny posture retained |
