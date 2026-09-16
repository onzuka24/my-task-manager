<script lang="ts">
  import { onMount } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { getCurrentWindow } from '@tauri-apps/api/window'

  type HotkeyStatus = {
    accelerator: string
    registered: boolean
    error: string | null
  }

  type OverlaySnapshot = {
    hotkey: HotkeyStatus
  }

  // TODO(CAP-2): ドメインモデル導入時に削除
  const DUMMY_NEXT_ACTION = '（まだドメインモデルを持たない — 常駐の骨格だけが立っている）'

  /** スナップショット取得の再試行回数と間隔。 */
  const SNAPSHOT_RETRIES = 10
  const SNAPSHOT_RETRY_INTERVAL_MS = 100

  let hotkey = $state<HotkeyStatus | null>(null)
  let snapshotError = $state<string | null>(null)

  function sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }

  // AD-3 (鮮度): 隠れている間に受け取ったイベントに依存せず、表示されるたびに
  // コマンドで完全なスナップショットを取得してから描画する。
  //
  // webview は常駐プロセスの setup が終わる前にも走りうる。その時点では常駐状態が
  // まだ確定しておらずコマンドは拒否を返す。一度の失敗で諦めるとスナップショットが
  // 永久に届かないため、短い間隔で再試行する。
  async function refresh(): Promise<void> {
    for (let attempt = 0; attempt < SNAPSHOT_RETRIES; attempt += 1) {
      try {
        const snapshot = await invoke<OverlaySnapshot>('get_overlay_snapshot')
        hotkey = snapshot.hotkey
        snapshotError = null
        return
      } catch (error) {
        if (attempt === SNAPSHOT_RETRIES - 1) {
          console.error('failed to fetch the overlay snapshot', error)
          // 再試行を使い切った。何も描かないと、ホットキーの登録失敗を知らせるために
          // 出したオーバーレイが空白のまま出ることになる。理由を必ず面に出す。
          snapshotError = String(error)
          return
        }
        await sleep(SNAPSHOT_RETRY_INTERVAL_MS)
      }
    }
  }

  /**
   * オーバーレイを閉じる。Esc とフォーカス離脱の共通経路である。
   *
   * 既定の経路はコマンドであり、隠すだけでなく直前に最前面だったアプリケーションへ
   * フォーカスを返す。コマンドが失敗したときは代替として webview から直接隠す —
   * 装飾なし・常に最前面のウィンドウが閉じられなくなることを避けるため。
   */
  async function close(): Promise<void> {
    try {
      await invoke('hide_overlay')
      return
    } catch (error) {
      console.error('failed to hide the overlay via the command', error)
    }
    try {
      await getCurrentWindow().hide()
      // 代替経路はコアを経由しないため、可視状態の記録が「表示中」のまま残る。
      // 放置すると次のホットキー押下が「隠す」に倒れて無反応になる。記録を戻す。
      await invoke('mark_overlay_hidden')
    } catch (error) {
      console.error('failed to hide the overlay window directly', error)
    }
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      event.preventDefault()
      void close()
    }
  }

  onMount(() => {
    void refresh()

    const unlisten = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) {
        // 表示のたびに完全なスナップショットを取り直す (AD-3 鮮度規則)。
        void refresh()
      } else {
        // 呼び出して使ったら消える一時的な面として扱う (AD-15)。
        // Esc と同じ経路を通す。
        void close()
      }
    })

    return () => {
      void unlisten.then((stop) => stop())
    }
  })
</script>

<svelte:window on:keydown={onKeydown} />

<main>
  {#if snapshotError}
    <p class="alert">
      常駐プロセスの状態を取得できなかった。ホットキーが使える状態かを確認できない。
      メニューバー項目の状態行を確認すること。
      <span class="detail">{snapshotError}</span>
    </p>
  {/if}

  {#if hotkey && !hotkey.registered}
    <p class="alert">
      {hotkey.accelerator} を登録できなかった。他のアプリケーションが同じキーを保持している。
      メニューバー項目からはいつでも状態を確認できる。
      {#if hotkey.error}<span class="detail">{hotkey.error}</span>{/if}
    </p>
  {/if}

  <p class="next-action">{DUMMY_NEXT_ACTION}</p>

  <p class="hint">Esc で閉じる{#if hotkey && hotkey.registered} · {hotkey.accelerator} で開閉{/if} · 終了はメニューバー項目から</p>
</main>

<style>
  main {
    height: 100%;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 0.75rem;
    padding: 1.75rem 2rem;
  }

  .next-action {
    margin: 0;
    font-size: 1.15rem;
    line-height: 1.6;
  }

  .hint {
    margin: 0;
    font-size: 0.78rem;
    color: var(--overlay-muted);
  }

  .alert {
    margin: 0;
    font-size: 0.85rem;
    line-height: 1.5;
    color: var(--overlay-alert);
  }

  .detail {
    display: block;
    color: var(--overlay-muted);
    font-size: 0.72rem;
  }
</style>
