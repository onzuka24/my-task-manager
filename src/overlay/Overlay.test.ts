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
let createOutcome = { moved: false }
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
      if (cmd === 'create_task') return createOutcome
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

function titleField(): HTMLInputElement {
  const input = document.querySelector('input.title')
  if (!input) throw new Error('タスクの題名の入力欄が無い')
  return input as HTMLInputElement
}

function stepsField(): HTMLTextAreaElement {
  const input = document.querySelector('textarea.steps')
  if (!input) throw new Error('ステップの入力欄が無い')
  return input as HTMLTextAreaElement
}

/** 入力欄へ本文を打ち込む。タイピングは打鍵に数えない (spec の Boundaries)。 */
function typeInto(field: HTMLInputElement | HTMLTextAreaElement, text: string): void {
  field.value = text
  field.dispatchEvent(new Event('input', { bubbles: true }))
}

/**
 * 指定のコマンドが決して応答しない IPC。**確定中の状態を作るために使う。**
 *
 * 進行中であることを表す旗 (`switching` / `creatingTask`) は、応答が返った時点で
 * 下ろされる。応答させなければ、その旗が立っている間の振る舞いを観測できる。
 */
function installHangingIPC(hanging: string): void {
  mockIPC(
    (cmd, args) => {
      invoked.push(cmd)
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> })
      if (cmd === hanging) return new Promise(() => {})
      if (cmd === 'get_overlay_snapshot') return snapshot
      if (cmd === 'switch_current_position') return switchOutcome
      if (cmd === 'create_task') return createOutcome
      return null
    },
    { shouldMockEvents: true },
  )
}

function noticeText(): string {
  return document.querySelector('[role="alert"]')?.textContent ?? ''
}

/** 作成の面へ入る。到達には明示的な打鍵が 1 回要る (FR-2 / AD-15)。 */
async function openCreation(): Promise<void> {
  press('n', { metaKey: true })
  await settle()
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
  createOutcome = { moved: false }
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

// --- タスクの作成 (CAP-4 / FR-4) ----------------------------------------------

test('初期表示に作成の面は現れない (FR-2 / AD-15)', () => {
  expect(document.querySelector('input.title')).toBeNull()
  expect(document.querySelector('textarea.steps')).toBeNull()
  // 到達の手がかりはあるが、面そのものは出ていない。
  expect(document.body.textContent ?? '').toContain('⌘N で新しいタスク')
})

test('⌘N で作成の面に入り、題名に入力位置がある', async () => {
  await openCreation()

  expect(document.querySelector('input.title')).not.toBeNull()
  expect(document.querySelector('textarea.steps')).not.toBeNull()
  expect(document.activeElement).toBe(titleField())
})

test('作成の面は既定表示と排他 — 既存のタスクもステップも並べない (FR-19)', async () => {
  await openCreation()

  const text = document.body.textContent ?? ''
  // 二つ以上のタスク名も、二つ以上の既存ステップの内容も同時に現れない。既定表示の
  // 次の一手ごと退く。
  expect(document.querySelector('.next-action')).toBeNull()
  expect(document.querySelector('.position')).toBeNull()
  expect(document.querySelector('textarea.note')).toBeNull()
  expect(text).not.toContain('3 段落目を書き直す')
})

test('未着手からも同じ打鍵で同じ面に入れる', async () => {
  snapshot = {
    ...SNAPSHOT,
    stepContent: null,
    stepOrdinal: null,
    stepCount: null,
    interruptionNote: null,
  }
  if (component) unmount(component)
  await mountOverlay()

  await openCreation()

  expect(document.querySelector('input.title')).not.toBeNull()
  expect(document.activeElement).toBe(titleField())
})

test('⌘Enter は作成のみ — 現在地は変わらず、オーバーレイも閉じない', async () => {
  await openCreation()
  typeInto(titleField(), '原稿')
  typeInto(stepsField(), '構成を決める\n下書きを書く\n推敲する')
  await settle()

  press('Enter', { metaKey: true })
  await settle()

  expect(argsOf('create_task')).toEqual({
    request: {
      title: '原稿',
      steps: '構成を決める\n下書きを書く\n推敲する',
      moveCurrentPosition: false,
    },
  })
  expect(invoked).not.toContain('hide_overlay')
  // 面を出て、取り直した既定表示へ戻る (AD-3 鮮度規則)。
  expect(invoked).toContain('get_overlay_snapshot')
  expect(document.querySelector('input.title')).toBeNull()
  expect(document.querySelector('.next-action')).not.toBeNull()
  // 面が畳まれても、作成できたことを確かめる手がかりが残る。
  expect(noticeText()).toContain('タスクを作成した')
  expect(noticeText()).toContain('現在地は変えていない')
})

test('⌘⇧Enter は作成して着手 — 現在地をその第 1 ステップへ置く', async () => {
  createOutcome = { moved: true }
  await openCreation()
  typeInto(titleField(), '原稿')
  typeInto(stepsField(), '構成を決める\n下書きを書く\n推敲する')
  await settle()

  press('Enter', { metaKey: true, shiftKey: true })
  await settle()

  expect(argsOf('create_task')).toEqual({
    request: {
      title: '原稿',
      steps: '構成を決める\n下書きを書く\n推敲する',
      moveCurrentPosition: true,
    },
  })
  expect(document.querySelector('input.title')).toBeNull()
  expect(noticeText()).toContain('現在地をその第 1 ステップへ移した')
})

test('着手できなかったときは、作成が済んでいることを述べて再確定を止める', async () => {
  // タスクは確定しているが現在地は動かなかった。黙って戻ると同じ入力が再確定され、
  // v1 では削除も到達もできない重複したタスクが生まれる。
  createOutcome = { moved: false }
  await openCreation()
  typeInto(titleField(), '原稿')
  typeInto(stepsField(), '構成を決める')
  await settle()

  press('Enter', { metaKey: true, shiftKey: true })
  await settle()

  expect(document.querySelector('input.title')).toBeNull()
  expect(noticeText()).toContain('タスクを作成した')
  expect(noticeText()).toContain('もう一度確定しないこと')
})

test('作成の成功は中断メモの下書きを消さない', async () => {
  typeNote('書きかけ')
  await settle()

  await openCreation()
  typeInto(titleField(), '原稿')
  typeInto(stepsField(), '構成を決める')
  await settle()

  press('Enter', { metaKey: true })
  await settle()

  // 下書きを捨ててよいのはフォーカス離脱と Esc だけである (refresh の規約)。
  expect(noteField().value).toBe('書きかけ')
})

test('警告が同時に出ても作成の面は切り取られず辿れる', async () => {
  snapshot = {
    ...SNAPSHOT,
    hotkey: { accelerator: 'Control + Option + Space', registered: false, error: '衝突' },
  }
  if (component) unmount(component)
  await mountOverlay()
  await openCreation()

  const main = document.querySelector('main') as HTMLElement
  expect(main.textContent ?? '').toContain('登録できなかった')
  expect(main.contains(titleField())).toBe(true)
  expect(main.contains(stepsField())).toBe(true)
  expect(main.querySelector('.hint')).not.toBeNull()
  // 固定サイズのウィンドウで溢れたとき、切り取るのではなく辿れること。
  expect(getComputedStyle(main).overflowY).toBe('auto')
})

test('切り替えの確定中は ⌘N で作成の面に入らない', async () => {
  installHangingIPC('switch_current_position')

  press('Enter')
  await settle()
  expect(invoked).toContain('switch_current_position')

  await openCreation()

  // 入れば、失敗したときの理由を読む機会が面ごと消える。
  expect(document.querySelector('input.title')).toBeNull()
})

test('作成の確定中は Esc で面を出ない', async () => {
  installHangingIPC('create_task')
  await openCreation()
  typeInto(titleField(), '原稿')
  typeInto(stepsField(), '構成を決める')
  await settle()

  press('Enter', { metaKey: true })
  await settle()
  expect(invoked).toContain('create_task')

  press('Escape')
  await settle()

  expect(document.querySelector('input.title')).not.toBeNull()
  expect(titleField().value).toBe('原稿')
  expect(invoked).not.toContain('hide_overlay')
})

test('確定中の二度目の ⌘Enter は二つ目のタスクを作らない', async () => {
  installHangingIPC('create_task')
  await openCreation()
  typeInto(titleField(), '原稿')
  typeInto(stepsField(), '構成を決める')
  await settle()

  press('Enter', { metaKey: true })
  await settle()
  press('Enter', { metaKey: true })
  await settle()

  expect(invoked.filter((cmd) => cmd === 'create_task')).toHaveLength(1)
})

test('押しっぱなしの ⌘Enter が、生まれたばかりのステップを切り替えない', async () => {
  await openCreation()
  typeInto(titleField(), '原稿')
  typeInto(stepsField(), '構成を決める')
  await settle()

  press('Enter', { metaKey: true })
  await settle()
  // 面は畳まれた。押し続けられた打鍵の反復がここへ落ちてくる。
  press('Enter', { metaKey: true, repeat: true })
  await settle()

  expect(invoked.filter((cmd) => cmd === 'create_task')).toHaveLength(1)
  expect(invoked).not.toContain('switch_current_position')
})

test('既定表示でも押しっぱなしの Enter は切り替えを撃たない', async () => {
  press('Enter', { repeat: true })
  await settle()

  expect(invoked).not.toContain('switch_current_position')
})

test('フォーカスを失った時点で作成の面は畳まれる', async () => {
  await openCreation()
  typeInto(titleField(), '書きかけ')
  await settle()

  await emit('tauri://blur', null)
  await settle()

  // 取得時にだけ捨てていると、フォーカスのイベントを伴わない再表示が作成の面の
  // まま初期表示になる (FR-2 が禁じる)。
  expect(document.querySelector('input.title')).toBeNull()
  expect(document.querySelector('.next-action')).not.toBeNull()
})

test('空行を含む入力は切り分けずそのまま送る — 落とす規則はコマンド境界が持つ', async () => {
  await openCreation()
  typeInto(titleField(), '原稿')
  typeInto(stepsField(), '構成を決める\n\n   \n下書きを書く\n\n')
  await settle()

  press('Enter', { metaKey: true })
  await settle()

  // フロントで行を切り分けて落とすと、規則が二箇所に分かれて片方だけが直る。
  expect(argsOf('create_task')).toEqual({
    request: {
      title: '原稿',
      steps: '構成を決める\n\n   \n下書きを書く\n\n',
      moveCurrentPosition: false,
    },
  })
})

test('題名が無ければ作成は失敗し、面は閉じず入力も残る', async () => {
  mockIPC(
    (cmd, args) => {
      invoked.push(cmd)
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> })
      if (cmd === 'create_task') throw new Error('題名が空である。タスクには題名が要る。')
      if (cmd === 'get_overlay_snapshot') return snapshot
      return null
    },
    { shouldMockEvents: true },
  )

  await openCreation()
  typeInto(stepsField(), '構成を決める')
  await settle()

  press('Enter', { metaKey: true })
  await settle()

  expect(document.querySelector('input.title')).not.toBeNull()
  expect(stepsField().value).toBe('構成を決める')
  const notice = document.querySelector('[role="alert"]')?.textContent ?? ''
  expect(notice).toContain('作成できなかった')
  expect(notice).toContain('題名が空である')
  expect(invoked).not.toContain('hide_overlay')
})

test('Esc は下書きを捨てて既定表示へ戻る — オーバーレイは閉じない', async () => {
  await openCreation()
  typeInto(titleField(), '書きかけ')
  typeInto(stepsField(), '書きかけの行')
  await settle()

  press('Escape')
  await settle()

  expect(invoked).not.toContain('hide_overlay')
  expect(invoked).not.toContain('create_task')
  expect(document.querySelector('input.title')).toBeNull()
  expect(document.querySelector('.next-action')).not.toBeNull()

  // 再度入っても入力は残っていない。
  await openCreation()
  expect(titleField().value).toBe('')
  expect(stepsField().value).toBe('')
})

test('フォーカスを失うと下書きは失われ、作成の面も畳まれる', async () => {
  await openCreation()
  typeInto(titleField(), '書きかけ')
  typeInto(stepsField(), '書きかけの行')
  await settle()

  await emit('tauri://blur', null)
  await settle()
  expect(invoked).not.toContain('create_task')

  await emit('tauri://focus', null)
  await settle()

  // 次の呼び出しは既定表示から始まる (FR-2)。
  expect(document.querySelector('input.title')).toBeNull()
  await openCreation()
  expect(titleField().value).toBe('')
  expect(stepsField().value).toBe('')
})

test('作成の面では素の Enter と Shift+Enter は改行であって確定ではない', async () => {
  await openCreation()
  typeInto(titleField(), '原稿')
  typeInto(stepsField(), '構成を決める')
  await settle()

  press('Enter')
  press('Enter', { shiftKey: true })
  await settle()

  // 2 行目を打とうとした打鍵でタスクが生まれてはならない。v1 に削除の経路は無い。
  expect(invoked).not.toContain('create_task')
  expect(document.querySelector('input.title')).not.toBeNull()
  // 案内と実際に効く打鍵が一致していること。
  const text = document.body.textContent ?? ''
  expect(text).toContain('⌘Enter で作成')
  expect(text).toContain('⌘⇧Enter で作成して着手')
})

test('作成の面で案内に無い修飾キー (Option / Control) では確定しない', async () => {
  await openCreation()
  press('Enter', { metaKey: true, altKey: true })
  press('Enter', { metaKey: true, ctrlKey: true })
  await settle()

  expect(invoked).not.toContain('create_task')
})

test('IME の変換確定の Enter を作成と取り違えない', async () => {
  await openCreation()

  press('Enter', { metaKey: true, isComposing: true } as KeyboardEventInit)
  await settle()

  expect(invoked).not.toContain('create_task')
})

test('IME の変換取り消しの Esc で作成の面を出ない', async () => {
  await openCreation()
  typeInto(titleField(), 'へんかんちゅう')
  await settle()

  press('Escape', { isComposing: true } as KeyboardEventInit)
  await settle()

  expect(document.querySelector('input.title')).not.toBeNull()
  expect(titleField().value).toBe('へんかんちゅう')
  expect(invoked).not.toContain('hide_overlay')
})

test('作成の面では Enter が切り替えを撃たない', async () => {
  await openCreation()

  press('Enter')
  press('Enter', { metaKey: true })
  await settle()

  expect(invoked).not.toContain('switch_current_position')
})

test('呼び出しから作成の確定までが 3 打鍵 — タイピングは数えない', async () => {
  // 1 打鍵目はホットキーによる呼び出し。webview からは観測できないため数に含める。
  const HOTKEY_PRESS = 1
  const observed: string[] = []
  const count = (event: KeyboardEvent) => observed.push(event.key)
  window.addEventListener('keydown', count)

  try {
    await openCreation()
    typeInto(titleField(), '原稿')
    typeInto(stepsField(), '構成を決める\n下書きを書く')
    await settle()

    press('Enter', { metaKey: true })
    await settle()

    expect(invoked).toContain('create_task')
    // 呼び出し + ⌘N + ⌘Enter。面へ入るのに要する明示操作は 1 回だけである。
    expect(HOTKEY_PRESS + observed.length).toBe(3)
  } finally {
    window.removeEventListener('keydown', count)
  }
})

test('作成の面は進捗率も件数も描かない (AD-15 / SM-C1)', async () => {
  await openCreation()

  const text = document.body.textContent ?? ''
  expect(text).not.toContain('%')
  expect(text).not.toContain('件')
  expect(document.querySelector('progress')).toBeNull()
  expect(document.querySelector('meter')).toBeNull()
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

test('コアが読めなくても作成の面には入れ、失敗の理由がそこに出る', async () => {
  const CORE_MISSING = '保存された状態を読み込めていない。'
  snapshot = {
    ...SNAPSHOT,
    stateError: CORE_MISSING,
    stepContent: null,
    stepOrdinal: null,
    stepCount: null,
    interruptionNote: null,
  }
  mockIPC(
    (cmd, args) => {
      invoked.push(cmd)
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> })
      if (cmd === 'create_task') throw new Error(CORE_MISSING)
      if (cmd === 'get_overlay_snapshot') return snapshot
      return null
    },
    { shouldMockEvents: true },
  )
  if (component) unmount(component)
  await mountOverlay()

  await openCreation()
  typeInto(titleField(), '原稿')
  typeInto(stepsField(), '構成を決める')
  await settle()

  press('Enter', { metaKey: true })
  await settle()

  // 面は閉じない。理由はその場に出る (I/O マトリクス「コア不在」)。
  expect(document.querySelector('input.title')).not.toBeNull()
  expect(document.querySelector('[role="alert"]')?.textContent ?? '').toContain(CORE_MISSING)
})
