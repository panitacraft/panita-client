# Isolated authentication network tests

Run from the repository root. Tests use synthetic accounts, loopback servers and an injected DNS failure. They never contact Microsoft, alter OS DNS/proxy/trust settings, or read real launcher accounts. Rust injection is test-only; rustls is a dev dependency already present transitively.

## Automated checks

```sh
cargo test -p launcher-core isolated_ --locked
python3 -B -m unittest discover -s tests -p test_network_fixture.py -v
```

Use `python` instead of `python3` on Windows if needed. Expected: six Rust tests and three Python tests pass.

- HTTP 404 produces no handoff/redirect or success. The pending synthetic login can be cancelled and restarted without replacing saved credentials.
- DNS failure returns a sanitized network error without querying a real resolver or retrying in the background. Explicit Microsoft Play may retry; offline authentication validation sends no request.
- A real TLS handshake rejects the untrusted certificate before HTTP credentials are sent. Separate controller checks verify account preservation and Microsoft Play rejection.
- Disposable positive-control clients explicitly trust the fixture to prove the server works. System/browser trust and production certificate verification remain unchanged.

## Manual browser checks

```sh
python3 tests/network_fixture.py
```

The fixture prints two loopback URLs with temporary ports, and stops after five minutes or Ctrl+C. Keep the terminal open during inspection.

1. Open the **HTTP 404** URL in Chrome or Firefox. Expect **404 - Synthetic callback unavailable**, with no launcher opening or login success.
2. Open the **Untrusted TLS** URL. Expect the browser certificate warning. Close the tab without bypassing the warning or installing the certificate.
3. For DNS, open `https://panita-fixture.invalid` without account/callback data. Expect a name-resolution error. Record proxy/VPN/browser-policy replacement pages as a limitation rather than passing the DNS case.
4. Close test tabs and stop the fixture. No settings need restoration.

In the standalone ZIP, run `python3 network_fixture.py` from the extracted folder. Do not paste real authorization responses into it. These pages cannot complete a real Microsoft operation. If a launcher login is pending, cancellation must remain usable. This checks missing handoff, not execution of the deployed callback page under a production outage. Record browser observations separately from automated coverage.

## Linux build-environment limitation (2026-10-16)

The current WSL full core suite fails protected atomic writes, including unchanged storage tests and a standalone synthetic `private-fs` durable write: `AtomicCommit`, unpublished, underlying ENOENT. Moving temporary files from tmpfs to native ext4 did not resolve it. Do not weaken protection or report storage-dependent Linux checks as passed. Transport and Python fixture results are separate. This does not supersede the owner's installed BOX evidence; investigate the build-environment failure before claiming full Linux automated regression success.
