import {bindings, defineConfig} from 'cf/config';
import * as entrypoint from './smoke.mjs' with {type:'cf-worker'};

// Staging deployment target is the bounded, stateless network smoke only.
// The live bridge remains disabled pending a separately reviewed config.
export default defineConfig({
  worker: {
    name: 'private-laptop-bridge-smoke',
    compatibilityDate: '2026-07-30',
    entrypoint,
    env: {
      ENABLED: bindings.text('false'),
      GATEWAY_HOST: bindings.text('private-bridge-smoke.invalid'),
      GATEWAY_ACCOUNT_ID: bindings.text(''), GATEWAY_POLICY_ID: bindings.text(''),
      START_MS: bindings.text(''), END_MS: bindings.text(''),
    },
  },
});
