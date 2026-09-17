/**
 * オーバーレイの配線を検証する。
 *
 * ここで見るのは「判断」ではなく「配線」である — Esc とフォーカス離脱が本当に
 * `hide_overlay` に繋がっているか、フォーカス取得がそれを呼んでいないか、打鍵が
 * `switch_current_position` に正しい引数で繋がっているか。Rust 側の純粋関数テストは
 * トグル規則とワイヤ契約を固定するが、`if (focused)` を反転させても、打鍵の分岐を
 * 取り違えても一つも落ちない。その穴を塞ぐ。
 */
import { emit } from '@tauri-apps/api/event'
import { clearMocks, mockIPC, mockWindows } from '@tauri-apps/api/mocks'
import { mount, unmount } from 'svelte'
import { afterEach, beforeEach, expect, test } from 'vitest'

import Overlay from './Overlay.svelte'

/** Overlay.svelte の再試行設定と一致させる。 */
const SNAPSHOT_RETRIES = 10
const SNAPSHOT_RETRY_INTERVAL_MS = 100

/** src-tauri/src/commands/mod.rs の `OverlaySnapshot` と 1:1。 */
const SNAPSHOT = {
  hotkey: {
    accelerator: 'Control + Option + Space',
    registered: true,
    error: null,
  },
  stateError: null,
  stepContent: '3 段落目を書き直す',
  stepOrdinal: 3,
  stepCount: 6,
  interruptionNote: '接続詞を整える途中',
}

/** `@tauri-apps/api` の mock が用意しないイベント内部実装の穴。 */
type EventPluginInternals = { unregisterListener?: (event: string, id: number) => void }

let invoked: string[] = []
let calls: { cmd: string; args: Record<string, unknown> }[] = []
let snapshot: Record<string, unknown> = { ...SNAPSHOT }
let switchOutcome = { moved: true }
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
function settle(ms = 0): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

/** 入力欄へ本文を打ち込む。タイピングは打鍵に数えない (spec の Boundaries)。 */
function typeNote(text: string): void {
  const input = noteField()
  input.value = text
  input.dispatchEvent(new Event('input', { bubbles: true }))
}

function installIPC(): void {
  mockIPC(
    (cmd, args) => {
      invoked.push(cmd)
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> })
      if (cmd === 'get_overlay_snapshot') return snapshot
      if (cmd === 'switch_current_position') return switchOutcome
      return null
    },
    { shouldMockEvents: true },
  )
}

function argsOf(cmd: string): Record<string, unknown> | undefined {
  return calls.find((call) => call.cmd === cmd)?.args
}

function noteField(): HTMLTextAreaElement {
  const input = document.querySelector('textarea.note')
  if (!input) throw new Error('中断メモの入力欄が無い')
  return input as HTMLTextAreaElement
}

function press(key: string, init: KeyboardEventInit = {}): void {
  window.dispatchEvent(new KeyboardEvent('keydown', { key, ...init }))
}

async function mountOverlay(): Promise<void> {
  document.body.innerHTML = '<div id="app"></div>'
  component = mount(Overlay, {
    target: document.getElementById('app') as HTMLElement,
  })
  await settle()
}

beforeEach(async () => {
  invoked = []
  calls = []
  snapshot = { ...SNAPSHOT }
  switchOutcome = { moved: true }
  mockWindows('main')
  installIPC()
  stubEventInternals()

  await mountOverlay()
  // 起動時のスナップショット取得は数えない。以降の呼び出しだけを見る。
  invoked = []
  calls = []
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
  press('Escape')
  await settle()

  expect(invoked).toContain('hide_overlay')
})

test('Esc 以外のキーでは閉じない', async () => {
  press('a')
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

  press('Escape')
  await settle()

  expect(invoked).toContain('hide_overlay')
  // 代替経路はコアを経由しないため、記録を戻さないと次の押下が死ぬ。
  expect(invoked).toContain('mark_overlay_hidden')
})

// --- 既定表示 (CAP-2 / FR-2) -------------------------------------------------

test('既定表示は次の一手と位置情報のみ — 他のステップ内容は現れない', () => {
  const text = document.body.textContent ?? ''

  expect(text).toContain('3 段落目を書き直す')
  expect(text).toContain('第 3 ステップ / 全 6 ステップ')
  // 全体像を持ち込んでいないこと。開示面 (CAP-9) は本スライスに属さない。
  expect(document.querySelectorAll('.next-action')).toHaveLength(1)
  expect(document.querySelectorAll('.position')).toHaveLength(1)
})

test('進捗率・バー・パーセントを描かない (AD-15)', () => {
  const text = document.body.textContent ?? ''

  expect(text).not.toContain('%')
  expect(document.querySelector('progress')).toBeNull()
  expect(document.querySelector('meter')).toBeNull()
})

test('未着手なら空欄にせず、その旨の 1 行を出す', async () => {
  snapshot = {
    ...SNAPSHOT,
    stepContent: null,
    stepOrdinal: null,
    stepCount: null,
    interruptionNote: null,
  }
  if (component) unmount(component)
  await mountOverlay()

  expect(document.body.textContent ?? '').toContain('未着手')
  // 離れるべき場所が無いのだから、メモの入力欄も出さない。
  expect(document.querySelector('textarea.note')).toBeNull()
})

// --- 再開時の提示 (CAP-8 / FR-7) ---------------------------------------------

test('記録済みの中断メモが追加操作なしに読め、カーソルは末尾にある', () => {
  const input = noteField()

  expect(input.value).toBe('接続詞を整える途中')
  expect(document.activeElement).toBe(input)
  expect(input.selectionStart).toBe(input.value.length)
  expect(input.selectionEnd).toBe(input.value.length)
})

test('メモが無いステップでは入力欄が空で始まる', async () => {
  snapshot = { ...SNAPSHOT, interruptionNote: null }
  if (component) unmount(component)
  await mountOverlay()

  expect(noteField().value).toBe('')
})

// --- 切り替え (CAP-7 / FR-4) -------------------------------------------------

test('メモを書き足して Enter — 今回書いた本文だけが送られる', async () => {
  typeNote('接続詞を整える途中 / 次は結論')
  await settle()

  press('Enter')
  await settle()

  expect(invoked).toContain('switch_current_position')
  expect(argsOf('switch_current_position')).toEqual({
    request: { note: '接続詞を整える途中 / 次は結論', declareCompletion: false },
  })
  // 移動できたので離脱する。
  expect(invoked).toContain('hide_overlay')
})

test('提示されたメモに触れずに Enter — 記入として送らない (SM-C3)', async () => {
  // 読み返しただけの切り替え。提示された本文をそのまま送り返すと、記入率が膨らむ。
  press('Enter')
  await settle()

  expect(argsOf('switch_current_position')).toEqual({
    request: { note: null, declareCompletion: false },
  })
  expect(invoked).toContain('hide_overlay')
})

test('⌘Enter は完了の宣言を伴う切り替え — 同じ一連の操作から 1 打鍵', async () => {
  typeNote('接続詞を整える途中 / 次は結論')
  await settle()

  press('Enter', { metaKey: true })
  await settle()

  expect(argsOf('switch_current_position')).toEqual({
    request: { note: '接続詞を整える途中 / 次は結論', declareCompletion: true },
  })
})

test('メモを空にしたまま確定しても切り替えは完了する', async () => {
  typeNote('')
  await settle()

  press('Enter')
  await settle()

  expect(argsOf('switch_current_position')).toEqual({
    request: { note: '', declareCompletion: false },
  })
  expect(invoked).toContain('hide_overlay')
})

test('呼び出しからメモ確定・離脱までが 5 打鍵以内 (FR-7)', async () => {
  // 1 打鍵目はホットキーによる呼び出し。webview からは観測できないため数に含める。
  const HOTKEY_PRESS = 1
  // **実際に窓へ届いた打鍵を数える。** テストが自分で持つ数ではなく、離脱まで何回
  // 押す必要があったかを見る — 二度の確定を要する実装になれば、1 回目の Enter では
  // `hide_overlay` に届かず落ちる。
  const observed: string[] = []
  const count = (event: KeyboardEvent) => observed.push(event.key)
  window.addEventListener('keydown', count)

  try {
    typeNote('接続詞を整える途中 / 次は結論')
    await settle()

    press('Enter')
    await settle()

    expect(invoked).toContain('switch_current_position')
    expect(invoked).toContain('hide_overlay')
    expect(HOTKEY_PRESS + observed.length).toBeLessThanOrEqual(5)
  } finally {
    window.removeEventListener('keydown', count)
  }
})

test('最終ステップでは離脱せず、確定したものを述べる — メモのみ', async () => {
  switchOutcome = { moved: false }
  typeNote('続きは明日')
  await settle()

  press('Enter')
  await settle()

  expect(invoked).toContain('switch_current_position')
  expect(invoked).not.toContain('hide_overlay')
  // 取り直してから示す (AD-3 鮮度規則)。
  expect(invoked).toContain('get_overlay_snapshot')
  const notice = document.querySelector('[role="alert"]')?.textContent ?? ''
  expect(notice).toContain('次のステップが無い')
  expect(notice).toContain('メモは記録した')
  expect(notice).not.toContain('完了')
})

test('最終ステップで完了も宣言したなら、そう述べる', async () => {
  switchOutcome = { moved: false }
  typeNote('続きは明日')
  await settle()

  press('Enter', { metaKey: true })
  await settle()

  const notice = document.querySelector('[role="alert"]')?.textContent ?? ''
  expect(notice).toContain('完了とメモは記録した')
})

test('最終ステップで何も宣言していないなら、記録は無かったと述べる', async () => {
  switchOutcome = { moved: false }

  press('Enter')
  await settle()

  const notice = document.querySelector('[role="alert"]')?.textContent ?? ''
  expect(notice).toContain('記録するものは無かった')
})

test('切り替えに失敗したら理由を面に出し、閉じない', async () => {
  mockIPC(
    (cmd, args) => {
      invoked.push(cmd)
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> })
      if (cmd === 'switch_current_position') throw new Error('書き込みに失敗した')
      if (cmd === 'get_overlay_snapshot') return snapshot
      return null
    },
    { shouldMockEvents: true },
  )
  invoked = []
  calls = []

  press('Enter')
  await settle()

  expect(invoked).not.toContain('hide_overlay')
  expect(document.querySelector('[role="alert"]')?.textContent ?? '').toContain(
    '切り替えを確定できなかった',
  )
})

test('IME の変換確定の Enter を切り替えと取り違えない', async () => {
  press('Enter', { isComposing: true } as KeyboardEventInit)
  await settle()

  expect(invoked).not.toContain('switch_current_position')
})

test('Shift+Enter は改行であり切り替えではない', async () => {
  press('Enter', { shiftKey: true })
  await settle()

  expect(invoked).not.toContain('switch_current_position')
})

test('案内に無い修飾キー (Option / Control) では切り替えない', async () => {
  press('Enter', { altKey: true })
  press('Enter', { ctrlKey: true })
  press('Enter', { metaKey: true, altKey: true })
  await settle()

  expect(invoked).not.toContain('switch_current_position')
  // 案内と実際に効く打鍵が一致していること。
  expect(document.body.textContent ?? '').toContain('⌘Enter で完了して切り替え')
})

test('IME の変換取り消しの Esc でオーバーレイを閉じない', async () => {
  typeNote('へんかんちゅう')
  await settle()

  press('Escape', { isComposing: true } as KeyboardEventInit)
  await settle()

  expect(invoked).not.toContain('hide_overlay')
  expect(noteField().value).toBe('へんかんちゅう')
})

test('未着手では切り替えを要求しない', async () => {
  snapshot = {
    ...SNAPSHOT,
    stepContent: null,
    stepOrdinal: null,
    stepCount: null,
    interruptionNote: null,
  }
  if (component) unmount(component)
  await mountOverlay()
  invoked = []

  press('Enter')
  await settle()

  expect(invoked).not.toContain('switch_current_position')
})

// --- 下書きの破棄 (AD-2 / AD-5) ----------------------------------------------

test('入力途中でフォーカスを失っても永続化せず、再表示で下書きは残らない', async () => {
  const input = noteField()
  input.value = '書きかけ'
  input.dispatchEvent(new Event('input', { bubbles: true }))
  await settle()

  await emit('tauri://blur', null)
  await settle()
  expect(invoked).not.toContain('switch_current_position')

  await emit('tauri://focus', null)
  await settle()

  expect(noteField().value).toBe('接続詞を整える途中')
})

// --- イベント (AD-3) ---------------------------------------------------------

test('current_position_changed を受けたらスナップショットを取り直す', async () => {
  snapshot = { ...SNAPSHOT, stepContent: '結論を書く', stepOrdinal: 4, interruptionNote: null }

  await emit('current_position_changed', null)
  await settle()

  expect(invoked).toContain('get_overlay_snapshot')
  const text = document.body.textContent ?? ''
  expect(text).toContain('結論を書く')
  expect(text).toContain('第 4 ステップ / 全 6 ステップ')
})

test('イベント由来の取り直しは入力途中の下書きを消さない', async () => {
  typeNote('書きかけ')
  await settle()
  const input = noteField()
  input.setSelectionRange(2, 2)

  snapshot = { ...SNAPSHOT, interruptionNote: '別の誰かが書いたメモ' }
  await emit('current_position_changed', null)
  await settle()

  // 下書きを捨ててよいのはフォーカス離脱と Esc だけである (I/O マトリクス)。
  expect(noteField().value).toBe('書きかけ')
  expect(noteField().selectionStart).toBe(2)
})

// --- スナップショットが取れないとき ------------------------------------------

test('スナップショットを取得できないままなら、古い現在地に対して切り替えを撃たない', async () => {
  mockIPC(
    (cmd, args) => {
      invoked.push(cmd)
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> })
      if (cmd === 'get_overlay_snapshot') throw new Error('常駐プロセスが応答しない')
      return null
    },
    { shouldMockEvents: true },
  )
  if (component) unmount(component)
  await mountOverlay()
  // 再試行 (10 回 × 100ms) を使い切るまで待つ。
  await settle(SNAPSHOT_RETRIES * SNAPSHOT_RETRY_INTERVAL_MS + 200)
  invoked = []

  press('Enter')
  await settle()

  expect(invoked).not.toContain('switch_current_position')
  expect(document.querySelector('textarea.note')).toBeNull()
  expect(document.body.textContent ?? '').toContain('状態を取得できなかった')
})

// --- コア不在 (I/O マトリクス「コア不在」) ------------------------------------

test('コアが読めなくてもホットキーの失敗は伝わる', async () => {
  snapshot = {
    ...SNAPSHOT,
    hotkey: { accelerator: 'Control + Option + Space', registered: false, error: '衝突' },
    stateError: '保存された状態を読み込めていない。',
    stepContent: null,
    stepOrdinal: null,
    stepCount: null,
    interruptionNote: null,
  }
  if (component) unmount(component)
  await mountOverlay()

  const text = document.body.textContent ?? ''
  expect(text).toContain('登録できなかった')
  expect(text).toContain('保存された状態を読み込めていない')
  // 状態を読めていないことを「未着手」として描かない。
  expect(text).not.toContain('未着手')
})
