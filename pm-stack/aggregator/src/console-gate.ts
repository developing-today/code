/** Add the console shortcut only to dashboard responses served through its public gate. */
export function throughConsoleGate(request: Request, publicHost = "pm.security.cab"): boolean {
  const requestHost = (request.headers.get("host") ?? "").trim().toLowerCase().replace(/:\d+$/, "");
  const expectedHost = publicHost.trim().toLowerCase().replace(/:\d+$/, "");
  return requestHost !== "" && requestHost === expectedHost;
}

export async function addConsoleShortcut(response: Response, request: Request, publicHost?: string): Promise<Response> {
  if (!throughConsoleGate(request, publicHost)) return response;
  const type = response.headers.get("content-type") ?? "";
  const path = new URL(request.url).pathname;
  let body: string;
  if (path === "/manifest.webmanifest" && type.includes("application/manifest+json")) {
    const manifest = JSON.parse(await response.text()) as Record<string, unknown>;
    const shortcuts = Array.isArray(manifest.shortcuts) ? manifest.shortcuts : [];
    if (!shortcuts.some((item) => item && typeof item === "object" && (item as { url?: unknown }).url === "/console/")) {
      manifest.shortcuts = [...shortcuts, { name: "Console", short_name: "Console", url: "/console/" }];
    }
    body = JSON.stringify(manifest, null, 2);
  } else if (type.includes("text/html")) {
    body = await response.text();
    body = body.replace("</nav>", '<a href="/console/">Console</a></nav>');
  } else {
    return response;
  }
  return new Response(body, { status: response.status, statusText: response.statusText, headers: response.headers });
}
