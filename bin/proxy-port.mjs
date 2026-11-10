// Strict parsing for the bin/* proxies' listen ports (#483).
//
// A bare parseInt is not a validator: 'abc' becomes NaN (an opaque
// ERR_SOCKET_BAD_PORT crash at listen time), '18744abc' and '18744.9'
// silently become 18744, and '0x4a38' becomes 0, which binds a random
// ephemeral port while every client keeps dialling the configured one.
// Every proxy parses its *_PROXY_PORT through here instead, so a bad
// value fails at startup with the env var and its value named.
//
// These scripts are launched by home-manager as
// `node ${../../bin}/<name>.mjs` (the whole bin/ directory is copied to
// the store), so this sibling import resolves both from a checkout and
// from the store path. Do not inline-copy this helper into a proxy.

export const parseProxyPortValue = (name, raw, fallback) => {
  if (raw === undefined || raw === '') return fallback;
  if (/^[0-9]+$/.test(raw)) {
    const port = Number(raw);
    if (port >= 1 && port <= 65535) return port;
  }
  throw new Error(
    `${name}="${raw}" is not a valid port (expected an integer 1-65535)`
  );
};

// Validate each comma-separated target; one bad entry must reject the whole list.
export const parseProxyPortListValue = (name, raw) => {
  if (raw === undefined || raw === '') {
    throw new Error(`${name} is required (comma list)`);
  }
  return raw.split(',').map((segment) => {
    const value = segment.trim();
    if (value === '') {
      throw new Error(`${name}="${raw}" contains an empty port segment`);
    }
    return parseProxyPortValue(name, value);
  });
};

// Strict port parsing for a host:port address taken from an env var
// (#494). The port is the segment after the last ':'; anything that is
// not exactly an integer 1-65535 is an error naming the variable and
// the full address, never a bare parseInt coercion.
export const parseAddressPortValue = (name, raw) => {
  const segment = typeof raw === 'string' ? raw.split(':').at(-1) : undefined;
  if (segment !== undefined && /^[0-9]+$/.test(segment)) {
    const port = Number(segment);
    if (port >= 1 && port <= 65535) return port;
  }
  throw new Error(
    `${name}="${raw}" is not a valid host:port address (expected a port integer 1-65535)`
  );
};

export const parseProxyPort = (name, fallback) => {
  try {
    return parseProxyPortValue(name, process.env[name], fallback);
  } catch (err) {
    console.error(err.message);
    process.exit(1);
  }
};

export const parseProxyPortList = (name) => {
  try {
    return parseProxyPortListValue(name, process.env[name]);
  } catch (err) {
    console.error(err.message);
    process.exit(1);
  }
};

const MAX_TIMEOUT_MS = 2147483647; // 2^31 - 1 (setTimeout 32-bit signed int limit)

export const parseProxyMillisValue = (name, raw, fallback) => {
  if (raw === undefined || raw === '') return fallback;
  if (/^[0-9]+$/.test(raw)) {
    const millis = Number(raw);
    if (millis >= 1 && millis <= MAX_TIMEOUT_MS) return millis;
  }
  throw new Error(
    `${name}="${raw}" is not a valid timeout (expected an integer 1-${MAX_TIMEOUT_MS} ms)`
  );
};

export const parseProxyMillis = (name, fallback) => {
  try {
    return parseProxyMillisValue(name, process.env[name], fallback);
  } catch (err) {
    console.error(err.message);
    process.exit(1);
  }
};
