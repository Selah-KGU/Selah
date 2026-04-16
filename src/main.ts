import App from "./App.svelte";
import { mount } from "svelte";

if (navigator.userAgent.includes('Windows')) {
  document.body.classList.add('platform-windows');
}

const app = mount(App, {
  target: document.getElementById("app")!,
});

export default app;
