import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { vitePages } from "@kingironman2011/vite-pages";
import { fileURLToPath, URL } from "node:url";

export default defineConfig({
  plugins: [react(), tailwindcss(), vitePages()],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  build: {
    rolldownOptions: {
      output: {
        codeSplitting: {
          groups: [
            {
              name: "react",
              test: /node_modules[\\/](?:react|react-dom|react-router|scheduler)(?:[\\/]|$)/,
              priority: 30,
            },
            {
              name: "data",
              test: /node_modules[\\/](?:@tanstack|react-hook-form|@hookform|zod)(?:[\\/]|$)/,
              priority: 20,
            },
            {
              name: "ui",
              test: /node_modules[\\/](?:@base-ui|lucide-react|clsx|tailwind-merge)(?:[\\/]|$)/,
              priority: 10,
            },
          ],
        },
      },
    },
  },
  server: {
    proxy: { "/api": "https://localhost:9080" },
  },
});
