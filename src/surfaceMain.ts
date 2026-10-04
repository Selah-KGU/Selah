import SurfaceApp from "./SurfaceApp.svelte";
import { mount } from "svelte";

const app = mount(SurfaceApp, {
  target: document.getElementById("app")!,
});

export default app;
