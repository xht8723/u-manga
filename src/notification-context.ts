import { getContext, setContext } from 'svelte';
import { Notifications } from './notifications';
const key = Symbol('U-Manga session notifications');
export function provideNotifications(controller: Notifications) { setContext(key, controller); }
export function useNotifications(): Notifications { return getContext<Notifications>(key); }
