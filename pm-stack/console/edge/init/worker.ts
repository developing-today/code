// The Worker before its first version: the object's class, so that its
// state has somewhere to live, and an answer that says there is nothing
// here yet.
//
// A version cannot be uploaded to a Worker that does not exist, and the
// storage behind a Durable Object class is only created by a deploy. So
// when there is no Worker, release.mjs deploys this one first and then
// uploads the real one over it as a version.
//
// It answers 503 with `ok: false` on purpose: the host's
// scripts/expose.py sends the hostname through the Worker only once its
// health check says `ok`, and an empty Worker must never get the
// hostname.
import { DurableObject } from "cloudflare:workers";

export class PmEdge extends DurableObject {}

export default {
  fetch(): Response {
    return new Response(JSON.stringify({ ok: false, edge: true, empty: true }), {
      status: 503,
      headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
    });
  },
};
