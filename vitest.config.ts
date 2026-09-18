import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vitest/config'

// オーバーレイの配線 (Esc・フォーカス離脱・代替経路) を検証するためだけの設定。
// 本体のビルドは vite.config.ts が受け持つ。
export default defineConfig({
  plugins: [svelte()],
  // Svelte 5 のコンポーネントを DOM 上で mount するため browser 条件で解決する。
  resolve: { conditions: ['browser'] },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
    // コンポーネントの `<style>` を jsdom に流し込む。固定サイズのウィンドウに
    // 収まりきらない内容へ到達できること (`overflow-y`) は配線と同じく壊れうるが、
    // これが無ければ `getComputedStyle` から一切見えない。
    css: true,
  },
})
