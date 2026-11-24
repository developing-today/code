import { defineWranglerConfig } from "wrangler/experimental-config";

// The cf CLI uses Wrangler's bundler bridge for the existing dist/ assets.
export default defineWranglerConfig({ assetsDirectory: "./dist" });
