<script lang="ts">
  /**
   * 二つの面のどちらを描くかをウィンドウのラベルで決める。
   *
   * # なぜ一つの束を二つのウィンドウが読むのか
   *
   * オーバーレイは通常ウィンドウ、**介入**は非活性パネルであり、フォーカスの挙動が
   * まったく違う (AD-6)。**兼ねてはならない**のはウィンドウの実装であって、フロント
   * エンドのビルドではない。入口を二つに割れば Svelte のランタイムが二重に積まれ、
   * 常駐 webview が二つになる本スライスで待機時のメモリ (AD-14) に効いてしまう。
   *
   * # ラベルは Rust 側の定数と 1:1 である
   *
   * `adapters/intervention/mod.rs` の `INTERVENTION_LABEL` と、tauri.conf.json の
   * `app.windows[].label` がここと一致していなければならない。**一致しなければ、
   * パネルにオーバーレイが描かれる** — 入力欄を持つ面がパネルとして出ることになり、
   * AD-6 が禁じた形そのものになる。
   */
  import { getCurrentWindow } from '@tauri-apps/api/window'

  import Intervention from './intervention/Intervention.svelte'
  import Overlay from './overlay/Overlay.svelte'

  /** src-tauri/src/adapters/intervention/mod.rs の `INTERVENTION_LABEL` と一致させる。 */
  const INTERVENTION_LABEL = 'intervention'

  /**
   * このウィンドウが介入パネルか。
   *
   * **ラベルを読めなければオーバーレイとして描く。** 逆に倒すと、読めなかったときに
   * 主たる面が失われる — オーバーレイはホットキーからの唯一の到達先である (CAP-1)。
   */
  let isIntervention = false
  try {
    isIntervention = getCurrentWindow().label === INTERVENTION_LABEL
  } catch (error) {
    console.error('failed to read the window label; falling back to the overlay', error)
  }
</script>

{#if isIntervention}
  <Intervention />
{:else}
  <Overlay />
{/if}
