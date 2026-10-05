import { writable } from 'svelte/store';

export const modalStack = writable<number[]>([]);
let sequence = 0;
export const modalIdentity = () => ++sequence;
export function registerModal(id: number) {
  modalStack.update(stack => [...stack, id]);
  return () => modalStack.update(stack => stack.filter(owner => owner !== id));
}
