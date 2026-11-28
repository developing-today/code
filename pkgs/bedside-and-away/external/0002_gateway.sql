CREATE TABLE gateway_nonces (
 nonce TEXT PRIMARY KEY NOT NULL,
 expires_at INTEGER NOT NULL
);
CREATE INDEX gateway_nonces_expiry ON gateway_nonces(expires_at);
