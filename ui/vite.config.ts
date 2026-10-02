import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  build: {
    // The 3D engine of the packaging model preview is a chunk of its own, about 580 kB, that
    // loads only once a model is shown.
    chunkSizeWarningLimit: 600,
  },
  server: {
    strictPort: true,
    host: "127.0.0.1",
  },
});
