sections: proxy
kind: Added
summary: configuration.md gains the proxy section (#2114): the new proxy.timeout setting (duration, 10m default, --proxy-timeout / MCPX_PROXY_TIMEOUT) that bounds how long the daemon waits for a chat-completions upstream, returning 502 when the deadline passes before response headers
