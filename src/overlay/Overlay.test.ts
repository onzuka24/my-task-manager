/**
 * オーバーレイの配線を検証する。
 *
 * ここで見るのは「判断」ではなく「配線」である — Esc とフォーカス離脱が本当に
 * `hide_overlay` に繋がっているか、フォーカス取得がそれを呼んでいないか。
 * Rust 側の純粋関数テストはトグル規則を固定するが、`if (focused)` を反転させても
 * 一つも落ちない。その穴を塞ぐ。
 */
import { emit } from '@tauri-apps/api/event'
import { clearMocks, mockIPC, mockWindows } from '@tauri-apps/api/mocks'
import { mount, unmount } from 'svelte'
import { afterEach, beforeEach, expect, test } from 'vitest'

import Overlay from './Overlay.svelte'

const SNAPSHOT = {
  hotkey: {
    accelerator: 'Control + Option + Space',
    registered: true,
    error: null,
  },
}

/** `@tauri-apps/api` の mock が用意しないイベント内部実装の穴。 */
type EventPluginInternals = { unregisterListener?: (event: string, id: number) => void }

let invoked: string[] = []
let component: Record<string, unknown> | undefined

/**
 * `mockIPC` は `listen` を模すが `unlisten` の内部実装を置かない。
 * 片付けのたびに未処理の rejection になるため、ここで埋める。
 */
function stubEventInternals(): void {
  const internals = (
    globalThis as unknown as {
      __TAURI_EVENT_PLUGIN_INTERNALS__: EventPluginInternals
    }
  ).__TAURI_EVENT_PLUGIN_INTERNALS__
  internals.unregisterListener = () => {}
}

/** イベントの配送とマイクロタスクが片付くまで待つ。 */
function settle(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0))
}

beforeEach(async () => {
  invoked = []
  mockWindows('main')
  mockIPC(
    (cmd) => {
      invoked.push(cmd)
      if (cmd === 'get_overlay_snapshot') return SNAPSHOT
      return null
    },
    { shouldMockEvents: true },
  )
  stubEventInternals()

  document.body.innerHTML = '<div id="app"></div>'
  component = mount(Overlay, {
    target: document.getElementById('app') as HTMLElement,
  })
  await settle()
  // 起動時のスナップショット取得は数えない。以降の呼び出しだけを見る。
  invoked = []
})

afterEach(async () => {
  if (component) unmount(component)
  component = undefined
  // 購読解除は非同期に走る。`clearMocks` がイベント内部実装を消す前に終わらせる。
  await settle()
  clearMocks()
})

test('フォーカスを失うと閉じる経路に入る', async () => {
  await emit('tauri://blur', null)
  await settle()

  expect(invoked).toContain('hide_overlay')
})

test('フォーカスを得ても閉じない — スナップショットを取り直すだけ', async () => {
  await emit('tauri://focus', null)
  await settle()

  expect(invoked).not.toContain('hide_overlay')
  expect(invoked).toContain('get_overlay_snapshot')
})

test('Esc で閉じる経路に入る', async () => {
  window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
  await settle()

  expect(invoked).toContain('hide_overlay')
})

test('Esc 以外のキーでは閉じない', async () => {
  window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }))
  await settle()

  expect(invoked).not.toContain('hide_overlay')
})

test('コマンドが失敗したら代替経路で隠し、コア側の可視状態の記録も戻す', async () => {
  mockIPC(
    (cmd) => {
      invoked.push(cmd)
      if (cmd === 'hide_overlay') throw new Error('window missing')
      return null
    },
    { shouldMockEvents: true },
  )
  invoked = []

  window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
  await settle()

  expect(invoked).toContain('hide_overlay')
  // 代替経路はコアを経由しないため、記録を戻さないと次の押下が死ぬ。
  expect(invoked).toContain('mark_overlay_hidden')
})
