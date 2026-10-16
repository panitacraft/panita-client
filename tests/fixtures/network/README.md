# Public, test-only localhost TLS fixture

The certificate and matching private key here are deliberately public synthetic test material. They are not application credentials, updater/release signing keys, or production TLS keys. Never reuse them outside loopback tests or install the certificate into a browser/OS trust store.

The self-signed certificate covers only `localhost` and `127.0.0.1`. Normal verification must reject it. A test-only positive control explicitly trusts it in an isolated HTTP client to prove the server is functional; production verification is never disabled or modified. The fixture expires in October 2036 and will need replacement then.

Generate replacement synthetic material with OpenSSL:

```sh
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout localhost-key.pem -out localhost-cert.pem -days 3650 \
  -subj '/CN=Panita isolated localhost fixture' \
  -addext 'subjectAltName=IP:127.0.0.1,DNS:localhost' \
  -addext 'basicConstraints=critical,CA:FALSE' \
  -addext 'keyUsage=critical,digitalSignature,keyEncipherment' \
  -addext 'extendedKeyUsage=serverAuth'
```

Automated Rust tests run with `cargo test -p launcher-core isolated_`. The manual browser fixture runs with `python tests/network_fixture.py`. Both bind exclusively to loopback, and neither uses real account data. DNS tests inject a resolver failure for a synthetic `.invalid` host without querying system DNS.
