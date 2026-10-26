import { Container, getContainer } from "@cloudflare/containers";

// Official Cloudflare hello-world image (Docker Hub cloudflare/hello-world,
// amd64 digest copied byte-for-byte into our account registry as :v1).
export class HelloContainer extends Container {
  defaultPort = 8000;
  sleepAfter = "2m";
}

// Probe: the Lab Ops Hub crawler image (Nix-built Python app) hosted in a
// FRESH container application, to isolate image vs application faults.
export class CrawlerProbe extends Container {
  defaultPort = 8080;
  sleepAfter = "2m";

  override onError(error: unknown) {
    console.log("CrawlerProbe error:", error);
  }
}

// Probe: official image but with instance_type "lite" (as Lab Ops Hub uses).
export class HelloLite extends Container {
  defaultPort = 8000;
  sleepAfter = "2m";
}

export default {
  async fetch(
    request: Request,
    env: {
      HELLO_CONTAINER: DurableObjectNamespace<HelloContainer>;
      CRAWLER_PROBE: DurableObjectNamespace<CrawlerProbe>;
      HELLO_LITE: DurableObjectNamespace<HelloLite>;
    },
  ): Promise<Response> {
    const pathname = new URL(request.url).pathname;

    if (pathname === "/worker-marker") {
      return new Response("served-by-worker\n");
    }

    if (pathname.startsWith("/probe")) {
      return getContainer(env.CRAWLER_PROBE, "probe-2").fetch(request);
    }

    if (pathname.startsWith("/lite")) {
      return getContainer(env.HELLO_LITE, "lite-1").fetch(request);
    }

    const name = pathname.startsWith("/name/") ? decodeURIComponent(pathname.slice("/name/".length)) : undefined;
    const container = name ? getContainer(env.HELLO_CONTAINER, name) : getContainer(env.HELLO_CONTAINER);
    return container.fetch(request);
  },
};
