"""Exercise the browser fixture without using a browser or production services."""
import http.client
import ssl
import threading
import unittest

from network_fixture import FIXTURES, servers


class BrowserFixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.http, cls.https = servers()
        for server in (cls.http, cls.https):
            threading.Thread(target=server.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        for server in (cls.http, cls.https):
            server.shutdown()
            server.server_close()

    def test_http_404_has_no_handoff_or_reflection(self):
        connection = http.client.HTTPConnection("127.0.0.1", self.http.server_port, timeout=5)
        self.addCleanup(connection.close)
        connection.request("GET", "/callback?code=synthetic-marker&state=synthetic-state")
        response = connection.getresponse()
        body = response.read()
        self.assertEqual(response.status, 404)
        self.assertIsNone(response.getheader("Location"))
        self.assertEqual(response.getheader("Cache-Control"), "no-store")
        for forbidden in (b"synthetic-marker", b"synthetic-state", b"panitacraft://", b"<script"):
            self.assertNotIn(forbidden, body)

    def test_untrusted_tls_is_rejected(self):
        context = ssl.create_default_context()
        connection = http.client.HTTPSConnection("127.0.0.1", self.https.server_port, context=context, timeout=5)
        self.addCleanup(connection.close)
        with self.assertRaises(ssl.SSLCertVerificationError):
            connection.request("GET", "/callback")

    def test_tls_fixture_positive_control(self):
        # Trust applies only to this disposable test client, never the system/browser.
        context = ssl.create_default_context(cafile=str(FIXTURES / "localhost-cert.pem"))
        connection = http.client.HTTPSConnection("127.0.0.1", self.https.server_port, context=context, timeout=5)
        self.addCleanup(connection.close)
        connection.request("GET", "/callback")
        response = connection.getresponse()
        self.assertEqual(response.status, 404)
        self.assertIn(b"Synthetic callback unavailable", response.read())


if __name__ == "__main__":
    unittest.main()
