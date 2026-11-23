import {bindings, defineConfig, exports} from 'cf/config';
import * as smoke from './smoke.mjs' with {type:'cf-worker'};
import * as bridge from './gateway.mjs' with {type:'cf-worker'};

const isolated = () => ({
  workersDev: false,
  previewUrls: false,
  domains: [],
  triggers: [],
  limits: {cpuMs: 10},
  observability: {enabled: false, logs: {enabled: false}, traces: {enabled: false}},
});

// The bridge mode is the sole real staging release definition. It remains disabled
// until owner-approved account, namespace, Gateway, and laptop preflight is complete.
// The default mode exists only for the separately managed synthetic smoke Worker.
export default defineConfig(({mode}) => mode === 'bridge' ? ({
  worker: {
    ...isolated(),
    name: 'private-laptop-bridge-staging',
    compatibilityDate: '2026-07-30',
    entrypoint: bridge,
    exports: {PrivateLaptopBridge: exports.durableObject({storage: 'sqlite'})},
    env: {
      ENABLED: bindings.text('false'),
      GATEWAY_HOST: bindings.text('private-bridge-staging.invalid'),
      GATEWAY_ACCOUNT_ID: bindings.text(''), GATEWAY_POLICY_ID: bindings.text(''),
      GATEWAY_IDENTITY_EMAIL: bindings.text(''), GATEWAY_POSTURE_ID: bindings.text(''),
      BRIDGE_REGISTRY: bindings.text(''),
      BRIDGE: bindings.durableObject({worker: 'private-laptop-bridge-staging', exportName: 'PrivateLaptopBridge'}),
    },
  },
}) : ({
  worker: {
    ...isolated(),
    name: 'private-laptop-bridge-smoke',
    compatibilityDate: '2026-07-30',
    entrypoint: smoke,
    env: {
      ENABLED: bindings.text('false'),
      GATEWAY_HOST: bindings.text('private-bridge-smoke.invalid'),
      GATEWAY_ACCOUNT_ID: bindings.text(''), GATEWAY_POLICY_ID: bindings.text(''),
      START_MS: bindings.text(''), END_MS: bindings.text(''),
    },
  },
}));
