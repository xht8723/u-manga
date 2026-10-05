import { call } from './bridge';
import { createCredentialAutosave } from './credential-autosave';
export const credentials = createCredentialAutosave((profile, value) =>
  call('secret_set', { profile, value }),
);
