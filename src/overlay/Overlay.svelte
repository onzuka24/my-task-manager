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

  let hotkey = $state<HotkeyStatus | null>(null)

  // AD-3 (鮮度): 隠れている間に受け取ったイベントに依存せず、表示されるたびに
  // コマンドで完全なスナップショットを取得してから描画する。
  async function refresh(): Promise<void> {
    try {
      const snapshot = await invoke<OverlaySnapshot>('get_overlay_snapshot')
      hotkey = snapshot.hotkey
    } catch (error) {
      console.error('failed to fetch the overlay snapshot', error)
    }
  }

  async function close(): Promise<void> {
    try {
      await invoke('hide_overlay')
    } catch (error) {
      console.error('failed to hide the overlay', error)
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
      if (focused) void refresh()
    })

    return () => {
      void unlisten.then((stop) => stop())
    }
  })
</script>

<svelte:window on:keydown={onKeydown} />

<main>
  {#if hotkey && !hotkey.registered}
    <p class="alert">
      {hotkey.accelerator} を登録できなかった。他のアプリケーションが同じキーを保持している。
      {#if hotkey.error}<span class="detail">{hotkey.error}</span>{/if}
    </p>
  {/if}

  <p class="next-action">{DUMMY_NEXT_ACTION}</p>

  <p class="hint">Esc で閉じる{#if hotkey && hotkey.registered} · {hotkey.accelerator} で開閉{/if}</p>
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
