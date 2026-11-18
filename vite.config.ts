import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    // 端口固定但会自动递增：端口被占用时 Vite 顺延到下一个，
    // 避免"Port already in use"导致前端起不来、窗口变空壳。
    port: 3100,
    strictPort: false,
  },
  build: {
    target: "chrome105",
    outDir: "dist",
  },
});
