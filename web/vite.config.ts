import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  server: { proxy: { "/api": { target: "http://127.0.0.1:8787", ws: true } } },
});
