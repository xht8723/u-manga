import { mount } from 'svelte';
import App from './SessionBoundary.svelte';
import './style.css';
import './library.css';
import './controls.css';
import './reader.css';
import './hosting.css';
// Suppress WebView/browser defaults without stopping our own book and chapter handlers.
document.addEventListener('contextmenu', (event) => event.preventDefault(), { capture: true });
mount(App, { target: document.getElementById('app')! });
