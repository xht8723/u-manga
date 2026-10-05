import rules from '../assets/thinking-models.json';
import type { Provider, ThinkingPolicy } from './types';
export function hostedThinking(p: Provider): ThinkingPolicy {
  const model = p.model.toLowerCase();
  const rule =
    p.service === 'llm' &&
    rules.find(
      (r) =>
        r.protocols.includes(p.protocol) &&
        (r.prefixes?.some((prefix) => model.startsWith(prefix)) ||
          r.models?.some(
            (name) =>
              model === name ||
              (model.startsWith(name) && /^-\d{4}-\d{2}-\d{2}$/.test(model.slice(name.length))),
          )),
    );
  return rule
    ? { state: rule.state as ThinkingPolicy['state'], on: rule.on, off: rule.off }
    : { state: 'managed', on: null, off: null };
}
