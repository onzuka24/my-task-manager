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

/**
 * 一覧の下敷きとなるコア状態。2 タスク・計 5 ステップ。現在地は第 2 タスクの第 3
 * ステップであり、第 1 タスクの第 2 ステップだけが完了している (spec の I/O マトリクス)。
 *
 * **ここから `get_disclosure_surface` の応答を組み立てる。** 固定の一覧を返すと、
 * 「開いているタスクの分だけを並べる」という射影の規則を、フロントの検査が素通りする
 * — どの要求にも同じ 5 ステップが返り、全部が同時に見えていても緑になる。
 */
const MANUSCRIPT = '0198f0e0-0000-7000-8000-0000000000a1'
const SHOPPING = '0198f0e0-0000-7000-8000-0000000000a2'

const TASKS = [
  {
    taskId: MANUSCRIPT,
    title: '原稿',
    steps: [
      { stepId: '0198f0e0-0000-7000-8000-000000000001', content: '構成を決める', completed: false },
      { stepId: '0198f0e0-0000-7000-8000-000000000002', content: '下書きを書く', completed: true },
    ],
  },
  {
    taskId: SHOPPING,
    title: '買い物',
    steps: [
      { stepId: '0198f0e0-0000-7000-8000-000000000003', content: '米', completed: false },
      { stepId: '0198f0e0-0000-7000-8000-000000000004', content: '味噌', completed: false },
      { stepId: '0198f0e0-0000-7000-8000-000000000005', content: '醤油', completed: false },
    ],
  },
]

/** 現在地のステップ。`null` なら未着手である。 */
let currentStepId: string | null = '0198f0e0-0000-7000-8000-000000000005'

/** 現在地を含むタスク。未着手ならどのタスクでもない。 */
function taskAtCurrentPosition(): string | null {
  return TASKS.find((task) => task.steps.some((step) => step.stepId === currentStepId))?.taskId ?? null
}

/**
 * src-tauri/src/commands/mod.rs の `disclosure_of` と同じ規則で一覧を組み立てる。
 *
 * **見出しは常にすべて並び、ステップは開いた 1 タスクの分だけが続く** (spec Boundaries)。
 * `openTaskId` が `null` なら開くのは現在地のタスクであり、未着手ならどれも開かない。
 */
function disclosureFor(openTaskId: string | null): Record<string, unknown> {
  const open = openTaskId ?? taskAtCurrentPosition()
  const rows: Record<string, unknown>[] = []
  for (const task of TASKS) {
    const opened = task.taskId === open
    rows.push({
      kind: 'task',
      taskId: task.taskId,
      title: task.title,
      open: opened,
      // **開閉とは別の欄である。** 閉じていても現在地を抱えていることを示す。
      holdsCurrentPosition: task.taskId === taskAtCurrentPosition(),
    })
    if (!opened) continue
    for (const step of task.steps) {
      rows.push({
        kind: 'step',
        stepId: step.stepId,
        content: step.content,
        completed: step.completed,
        current: step.stepId === currentStepId,
      })
    }
  }
  return { stateError: null, rows }
}

/** `@tauri-apps/api` の mock が用意しないイベント内部実装の穴。 */
type EventPluginInternals = { unregisterListener?: (event: string, id: number) => void }

let invoked: string[] = []
let calls: { cmd: string; args: Record<string, unknown> }[] = []
let snapshot: Record<string, unknown> = { ...SNAPSHOT }
let switchOutcome = { moved: true }
let createOutcome = { moved: false }
/**
 * 一覧の応答を差し替える。**空の一覧・コア不在のように、状態から組み立てられない
 * 応答だけがここを使う。** `null` のときは [`disclosureFor`] が要求に応じて組み立てる。
 */
let disclosureOverride: Record<string, unknown> | null = null
/** 一覧の取得を失敗させる旗。**取り直しの契機を保ったまま失敗を起こすために使う。** */
let disclosureFails = false
/** 一覧の応答を握り、任意の時点で返すための仕掛け。遅れて着く応答を作る。 */
let holdDisclosure = false
let releaseDisclosure: ((surface: Record<string, unknown>) => void) | null = null
let selectOutcome = { moved: true }
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
      if (cmd === 'get_disclosure_surface') {
        if (disclosureFails) throw new Error('常駐プロセスが応答しない')
        if (holdDisclosure) {
          return new Promise((resolve) => {
            releaseDisclosure = resolve as (surface: Record<string, unknown>) => void
          })
        }
        return surfaceFor(args)
      }
      if (cmd === 'select_step') return selectOutcome
      return null
    },
    { shouldMockEvents: true },
  )
}

function argsOf(cmd: string): Record<string, unknown> | undefined {
  return calls.find((call) => call.cmd === cmd)?.args
}

/** 直近の呼び出しの引数。**開き直しの要求は毎回変わる** — 最初の 1 回では見えない。 */
function lastArgsOf(cmd: string): Record<string, unknown> | undefined {
  return [...calls].reverse().find((call) => call.cmd === cmd)?.args
}

/**
 * コマンドの引数から一覧の応答を作る。
 *
 * **要求された `openTaskId` を実際に読む。** 無視して固定の一覧を返せば、見出しの確定が
 * 何も変えていなくても検査は緑になる。
 */
function surfaceFor(args: unknown): Record<string, unknown> {
  if (disclosureOverride) return disclosureOverride
  const request = ((args ?? {}) as { request?: { openTaskId?: string | null } }).request ?? {}
  return disclosureFor(request.openTaskId ?? null)
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
      if (cmd === 'get_disclosure_surface') {
        if (disclosureFails) throw new Error('常駐プロセスが応答しない')
        if (holdDisclosure) {
          return new Promise((resolve) => {
            releaseDisclosure = resolve as (surface: Record<string, unknown>) => void
          })
        }
        return surfaceFor(args)
      }
      if (cmd === 'select_step') return selectOutcome
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

/** 開示面へ入る。到達には明示的な打鍵が 1 回要る (FR-19 / FR-2 / AD-15)。 */
async function openDisclosure(): Promise<void> {
  press('l', { metaKey: true })
  await settle()
}

function stepRows(): HTMLButtonElement[] {
  return [...document.querySelectorAll<HTMLButtonElement>('.step-row')]
}

function rowTexts(): string[] {
  return stepRows().map((row) => row.textContent?.trim() ?? '')
}

/** 見出しの行。**常にすべて現れる** — 隠れるのはステップだけである。 */
function headings(): HTMLButtonElement[] {
  return [...document.querySelectorAll<HTMLButtonElement>('.task-heading')]
}

/** 見出しの題名。**印は含めない** — 印そのものは別の検査が見る。 */
function headingTitle(row: HTMLElement): string {
  return row.querySelector('.content')?.textContent?.trim() ?? ''
}

function headingTexts(): string[] {
  return headings().map(headingTitle)
}

/** 開いている見出し。**高々一つである** (spec Boundaries)。 */
function openedHeadings(): string[] {
  return headings()
    .filter((row) => row.getAttribute('aria-expanded') === 'true')
    .map(headingTitle)
}

/** **現在地**を抱えると示している見出し。**高々一つである** (CAP-6 / FR-6)。 */
function headingsWithTheCurrentPosition(): string[] {
  return headings()
    .filter((row) => row.getAttribute('aria-current') === 'location')
    .map(headingTitle)
}

/** 入力位置を、指定の鍵を持つ行まで ↑ で運ぶ。到達できなければ投げる。 */
async function moveSelectionTo(rowKey: string): Promise<void> {
  for (let i = 0; i < 16; i += 1) {
    if ((document.activeElement as HTMLElement)?.dataset.rowKey === rowKey) return
    press('ArrowUp')
    await settle()
  }
  throw new Error(`${rowKey} へ入力位置が届かない`)
}

function press(key: string, init: KeyboardEventInit = {}): void {
  window.dispatchEvent(new KeyboardEvent('keydown', { key, ...init }))
}

/** 打鍵を飲んだか (`preventDefault`) まで見たいときに使う。 */
function pressCancelable(key: string, init: KeyboardEventInit = {}): KeyboardEvent {
  const event = new KeyboardEvent('keydown', { key, cancelable: true, ...init })
  window.dispatchEvent(event)
  return event
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
  currentStepId = '0198f0e0-0000-7000-8000-000000000005'
  disclosureOverride = null
  disclosureFails = false
  holdDisclosure = false
  releaseDisclosure = null
  selectOutcome = { moved: true }
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


// --- 開示面 (CAP-9 / FR-19) ---------------------------------------------------

test('初期表示に開示面は現れない (FR-19 / FR-2 / AD-15)', () => {
  expect(document.querySelector('.disclosure')).toBeNull()
  expect(stepRows()).toHaveLength(0)
  // 隠れている間に一覧を取りに行かない。
  expect(invoked).not.toContain('get_disclosure_surface')
  // 到達の手がかりはあるが、面そのものは出ていない。
  expect(document.body.textContent ?? '').toContain('⌘L で一覧')
})

test('⌘L で一覧に入り、現在地の行に入力位置がある', async () => {
  await openDisclosure()

  // 表示のたびに完全なスナップショットを取得してから描く (AD-3 鮮度規則)。
  expect(invoked).toContain('get_disclosure_surface')
  // **開くタスクを決めるのはコアである。** 面を開いた時点の要求は `null` であり、
  // 前回どのタスクを開いていたかを持ち越さない。
  expect(argsOf('get_disclosure_surface')).toEqual({ request: { openTaskId: null } })
  expect(document.querySelector('.disclosure')).not.toBeNull()
  const here = stepRows().find((row) => row.getAttribute('aria-current') === 'step')
  expect(here?.dataset.stepId).toBe('0198f0e0-0000-7000-8000-000000000005')
  expect(document.activeElement).toBe(here)
})

test('見出しはすべて現れるが、ステップは現在地のタスクの分だけである (設計上の賭け #1)', async () => {
  await openDisclosure()

  // 見出しは 2 件とも現れる。
  expect(headingTexts()).toEqual(['原稿', '買い物'])
  // **開いているのは現在地のタスクだけである。**
  expect(openedHeadings()).toEqual(['買い物'])
  // 全部のステップが同時に見えてはならない。並ぶのは開いた 1 タスクの分だけである。
  expect(stepRows()).toHaveLength(3)
  expect(rowTexts()[0]).toContain('米')
  expect(rowTexts()[1]).toContain('味噌')
  expect(rowTexts()[2]).toContain('醤油')
})

test('全部のステップが同時に見える形にならない — 閉じたタスクの中身は現れない', async () => {
  await openDisclosure()

  const text = document.body.textContent ?? ''
  expect(text).toContain('原稿')
  expect(text).not.toContain('構成を決める')
  expect(text).not.toContain('下書きを書く')
  expect(stepRows()).toHaveLength(3)
})

test('見出しを確定するとそのタスクが開き、直前のタスクが閉じる', async () => {
  await openDisclosure()
  expect(openedHeadings()).toEqual(['買い物'])

  await moveSelectionTo(`task:${MANUSCRIPT}`)
  press('Enter')
  await settle()

  // 開くタスクが変わっただけで、一覧をコアから作り直す。
  expect(lastArgsOf('get_disclosure_surface')).toEqual({
    request: { openTaskId: MANUSCRIPT },
  })
  expect(openedHeadings()).toEqual(['原稿'])
  expect(stepRows()).toHaveLength(2)
  expect(rowTexts()[0]).toContain('構成を決める')
  expect(rowTexts()[1]).toContain('下書きを書く')
  // 直前に開いていたタスクのステップは消える。
  expect(document.body.textContent ?? '').not.toContain('醤油')
  // 見出しは 2 件とも残る。面も開いたままである。
  expect(headingTexts()).toEqual(['原稿', '買い物'])
  expect(document.querySelector('.disclosure')).not.toBeNull()
})

test('見出しの確定は現在地を動かさず、履歴も増やさない', async () => {
  await openDisclosure()
  invoked = []

  await moveSelectionTo(`task:${MANUSCRIPT}`)
  press('Enter')
  await settle()

  // 何も書かない。切り替えの儀式も、一覧からの移動も撃たない。
  expect(invoked).not.toContain('select_step')
  expect(invoked).not.toContain('switch_current_position')
  // 既定表示のスナップショットも取り直さない — 現在地は動いていない。
  expect(invoked).not.toContain('get_overlay_snapshot')
  expect(invoked).not.toContain('hide_overlay')
})

test('確定した見出しに入力位置が留まる — 開いた行の上に落ちない', async () => {
  await openDisclosure()

  await moveSelectionTo(`task:${MANUSCRIPT}`)
  press('Enter')
  await settle()

  expect((document.activeElement as HTMLElement)?.dataset.rowKey).toBe(`task:${MANUSCRIPT}`)
  // そこから ↓ で、開いたばかりのステップへ入れる。
  press('ArrowDown')
  await settle()
  expect((document.activeElement as HTMLElement)?.dataset.stepId).toBe(
    '0198f0e0-0000-7000-8000-000000000001',
  )
})

test('開いたタスクの完了したステップに素朴な印が付く', async () => {
  await openDisclosure()
  await moveSelectionTo(`task:${MANUSCRIPT}`)
  press('Enter')
  await settle()

  expect(rowTexts()[0]).toContain('構成を決める')
  // 完了した行にだけ印が付く。
  expect(rowTexts()[1]).toContain('✓')
  expect(rowTexts()[0]).not.toContain('✓')
})

test('別のタスクを開いても、現在地がどこにあるかは見出しが示す', async () => {
  await openDisclosure()

  await moveSelectionTo(`task:${MANUSCRIPT}`)
  press('Enter')
  await settle()

  // 現在地のステップは閉じたタスクの中にあり、行としては現れない。
  expect(document.body.textContent ?? '').not.toContain('醤油')
  expect(stepRows().filter((row) => row.getAttribute('aria-current') === 'step')).toHaveLength(0)
  // **それでも現在地は分かる。** 印は閉じている見出しの側に付く。
  expect(headingsWithTheCurrentPosition()).toEqual(['買い物'])
  const marked = headings().find((row) => row.getAttribute('aria-current') === 'location')
  expect(marked?.querySelector('.here')?.textContent?.trim()).toBe('●')
  // 開いているのは別のタスクである — 印と開閉を取り違えていない。
  expect(openedHeadings()).toEqual(['原稿'])
  expect(marked?.getAttribute('aria-expanded')).toBe('false')
})

test('現在地の印は一つだけである (CAP-6 / FR-6)', async () => {
  await openDisclosure()

  // 開いているタスクが現在地を抱えているときも、印は見出しに付く。
  expect(headingsWithTheCurrentPosition()).toEqual(['買い物'])
  // 印の付いた見出しは一つ、印の付いた行も一つ。二つの印は別の意味を持つ。
  expect(headings().filter((row) => row.querySelector('.here')?.textContent?.trim())).toHaveLength(1)
  expect(stepRows().filter((row) => row.getAttribute('aria-current') === 'step')).toHaveLength(1)
})

test('未着手ならどのタスクも開かない — 見出しだけが現れる', async () => {
  currentStepId = null
  await openDisclosure()

  expect(headingTexts()).toEqual(['原稿', '買い物'])
  expect(openedHeadings()).toEqual([])
  expect(stepRows()).toHaveLength(0)
  // **無い現在地を描かない。** 印を付ければ、まだ始めていないことが「ここにいる」と
  // して静かに描かれる。
  expect(headingsWithTheCurrentPosition()).toEqual([])
  expect(headings().filter((row) => row.querySelector('.here')?.textContent?.trim())).toHaveLength(0)
  // 入力位置は先頭の見出しにある。
  expect((document.activeElement as HTMLElement)?.dataset.rowKey).toBe(`task:${MANUSCRIPT}`)
})

test('未着手でも見出しを確定すれば、着手せずに作ったタスクへ到達できる', async () => {
  currentStepId = null
  await openDisclosure()

  press('Enter')
  await settle()

  expect(openedHeadings()).toEqual(['原稿'])
  expect(rowTexts()[0]).toContain('構成を決める')

  // そのステップを確定すれば現在地がそこへ移る — 到達不能が解消する経路である。
  press('ArrowDown')
  await settle()
  press('Enter')
  await settle()
  expect(argsOf('select_step')).toEqual({
    request: { stepId: '0198f0e0-0000-7000-8000-000000000001' },
  })
})

test('現在地の行がそれと分かる — 印は一つだけである (FR-6)', async () => {
  await openDisclosure()

  const marked = stepRows().filter((row) => row.getAttribute('aria-current') === 'step')
  expect(marked).toHaveLength(1)
  expect(marked[0].textContent ?? '').toContain('醤油')
  expect(marked[0].textContent ?? '').toContain('▸')
})

test('一覧は進捗率も件数も総数も描かない (AD-15)', async () => {
  await openDisclosure()

  const text = document.body.textContent ?? ''
  expect(text).not.toContain('%')
  expect(text).not.toContain('件')
  expect(text).not.toContain('全 5')
  expect(text).not.toContain('ステップ /')
  expect(document.querySelector('progress')).toBeNull()
  expect(document.querySelector('meter')).toBeNull()
  // 見出しの印も位置情報である — 数も割合も伴わない。
  expect(headings().map((row) => row.querySelector('.here')?.textContent?.trim() ?? '')).toEqual([
    '',
    '●',
  ])
})

test('一覧は並べ替え・改名・削除の手がかりを持たない (SPEC 非目標)', async () => {
  await openDisclosure()

  // 面に入力欄が無いことが、編集の面へ滑り出していないことの実体である。
  expect(document.querySelector('input')).toBeNull()
  expect(document.querySelector('textarea')).toBeNull()
})

test('↑↓ で入力位置が行の間を動く — 見出しにも止まる', async () => {
  await openDisclosure()

  press('ArrowUp')
  await settle()
  expect((document.activeElement as HTMLElement)?.dataset.stepId).toBe(
    '0198f0e0-0000-7000-8000-000000000004',
  )

  // **見出しは選べる行である。** 読み飛ばすと、閉じたタスクを開く手段が無くなる。
  press('ArrowUp')
  press('ArrowUp')
  await settle()
  expect((document.activeElement as HTMLElement)?.dataset.rowKey).toBe(`task:${SHOPPING}`)

  press('ArrowUp')
  await settle()
  expect((document.activeElement as HTMLElement)?.dataset.rowKey).toBe(`task:${MANUSCRIPT}`)

  press('ArrowDown')
  await settle()
  expect((document.activeElement as HTMLElement)?.dataset.rowKey).toBe(`task:${SHOPPING}`)
})

test('端では留まる — 回り込まない', async () => {
  await openDisclosure()

  for (let i = 0; i < 8; i += 1) press('ArrowUp')
  await settle()
  // 先頭は第 1 タスクの見出しである。
  expect((document.activeElement as HTMLElement)?.dataset.rowKey).toBe(`task:${MANUSCRIPT}`)

  for (let i = 0; i < 8; i += 1) press('ArrowDown')
  await settle()
  expect((document.activeElement as HTMLElement)?.dataset.stepId).toBe(
    '0198f0e0-0000-7000-8000-000000000005',
  )
})

test('別のステップを選ぶと現在地が移り、面は閉じて既定表示へ戻る', async () => {
  snapshot = { ...SNAPSHOT, stepContent: '味噌', stepOrdinal: 2, interruptionNote: null }
  await openDisclosure()

  press('ArrowUp')
  await settle()
  press('Enter')
  await settle()

  expect(argsOf('select_step')).toEqual({
    request: { stepId: '0198f0e0-0000-7000-8000-000000000004' },
  })
  // 切り替えの儀式 (CAP-7) は撃たない。移動と履歴はコア側の単一のトランザクションである。
  expect(invoked).not.toContain('switch_current_position')
  // オーバーレイは閉じない。移った先の次の一手をそのまま見せる (FR-8)。
  expect(invoked).not.toContain('hide_overlay')
  // 取り直した既定表示がそのステップを示す (AD-3 鮮度規則)。
  expect(invoked).toContain('get_overlay_snapshot')
  expect(document.querySelector('.disclosure')).toBeNull()
  expect(document.body.textContent ?? '').toContain('味噌')
})

test('中断メモも完了もこの経路からは送らない', async () => {
  await openDisclosure()
  press('ArrowUp')
  await settle()
  press('Enter')
  await settle()

  const request = (argsOf('select_step') ?? {}).request as Record<string, unknown>
  // 欄が無いことが「機会を与えていない」の実体である (SM-C3)。
  expect(Object.keys(request)).toEqual(['stepId'])
})

test('現在地そのものを選んでも、切り替えの儀式は起きない', async () => {
  selectOutcome = { moved: false }
  await openDisclosure()

  press('Enter')
  await settle()

  // 何も書かないのはコアの判断である。フロントは現在地の行をそのまま送る。
  expect(argsOf('select_step')).toEqual({
    request: { stepId: '0198f0e0-0000-7000-8000-000000000005' },
  })
  expect(invoked).not.toContain('switch_current_position')
  expect(document.querySelector('.disclosure')).toBeNull()
})

test('一覧の ⌘Enter は完了を宣言しない — どこにも束縛されていない', async () => {
  await openDisclosure()

  press('Enter', { metaKey: true })
  press('Enter', { shiftKey: true })
  press('Enter', { altKey: true })
  press('Enter', { ctrlKey: true })
  await settle()

  expect(invoked).not.toContain('select_step')
  expect(invoked).not.toContain('switch_current_position')
  // 案内と実際に効く打鍵が一致していること。
  expect(document.body.textContent ?? '').toContain('Enter でここへ現在地を移す')
})

test('見出しの上では案内が変わる — 現在地を移すとは述べない', async () => {
  await openDisclosure()
  expect(document.body.textContent ?? '').toContain('Enter でここへ現在地を移す')

  await moveSelectionTo(`task:${MANUSCRIPT}`)

  const text = document.body.textContent ?? ''
  expect(text).toContain('Enter でこのタスクのステップを見る')
  expect(text).not.toContain('Enter でここへ現在地を移す')
})

test('見出しの上でも ⌘Enter はどこにも束縛されていない', async () => {
  await openDisclosure()
  await moveSelectionTo(`task:${MANUSCRIPT}`)
  invoked = []

  press('Enter', { metaKey: true })
  press('Enter', { shiftKey: true })
  await settle()

  expect(invoked).not.toContain('get_disclosure_surface')
  expect(invoked).not.toContain('select_step')
})

test('click は選ぶだけで、見出しを開かない', async () => {
  await openDisclosure()
  invoked = []

  headings()[0].click()
  await settle()

  // 押した行が選ばれるだけである。開くのは Enter の仕事である。
  expect((document.activeElement as HTMLElement)?.dataset.rowKey).toBe(`task:${MANUSCRIPT}`)
  expect(openedHeadings()).toEqual(['買い物'])
  expect(invoked).not.toContain('get_disclosure_surface')
})

test('押しっぱなしの Enter は二度確定しない', async () => {
  await openDisclosure()

  press('Enter')
  press('Enter', { repeat: true })
  await settle()

  expect(invoked.filter((cmd) => cmd === 'select_step')).toHaveLength(1)
})

test('IME の変換確定の Enter を選択と取り違えない', async () => {
  await openDisclosure()

  press('Enter', { isComposing: true } as KeyboardEventInit)
  await settle()

  expect(invoked).not.toContain('select_step')
})

test('Esc は既定表示へ戻る — オーバーレイは閉じない', async () => {
  await openDisclosure()

  press('Escape')
  await settle()

  expect(invoked).not.toContain('hide_overlay')
  expect(invoked).not.toContain('select_step')
  expect(document.querySelector('.disclosure')).toBeNull()
  expect(document.querySelector('.next-action')).not.toBeNull()
  // 入力位置は中断メモの欄へ返る。
  expect(document.activeElement).toBe(noteField())
})

test('一覧を出したままオーバーレイを閉じると、次回の呼び出しは初期表示である', async () => {
  await openDisclosure()
  expect(document.querySelector('.disclosure')).not.toBeNull()

  await emit('tauri://blur', null)
  await settle()
  // 閉じた時点で破棄される (FR-19)。
  expect(document.querySelector('.disclosure')).toBeNull()

  await emit('tauri://focus', null)
  await settle()

  expect(document.querySelector('.disclosure')).toBeNull()
  expect(document.querySelector('.next-action')).not.toBeNull()
})

test('開いていたタスクも持ち越さない — 次回の一覧は現在地のタスクが開く', async () => {
  await openDisclosure()
  await moveSelectionTo(`task:${MANUSCRIPT}`)
  press('Enter')
  await settle()
  expect(openedHeadings()).toEqual(['原稿'])

  await emit('tauri://blur', null)
  await settle()
  await emit('tauri://focus', null)
  await settle()
  calls = []

  await openDisclosure()

  expect(argsOf('get_disclosure_surface')).toEqual({ request: { openTaskId: null } })
  expect(openedHeadings()).toEqual(['買い物'])
})

test('Esc で戻ってから開き直しても、開くのは現在地のタスクである', async () => {
  await openDisclosure()
  await moveSelectionTo(`task:${MANUSCRIPT}`)
  press('Enter')
  await settle()

  press('Escape')
  await settle()
  calls = []
  await openDisclosure()

  expect(argsOf('get_disclosure_surface')).toEqual({ request: { openTaskId: null } })
  expect(openedHeadings()).toEqual(['買い物'])
})

test('タスクが 1 個も無ければ空欄にせず、その旨の 1 行を出す', async () => {
  disclosureOverride = { stateError: null, rows: [] }
  await openDisclosure()

  expect(stepRows()).toHaveLength(0)
  expect(headings()).toHaveLength(0)
  expect(document.body.textContent ?? '').toContain('まだタスクが無い')

  // 選べる行が無いのだから、Enter は何も撃たない。
  press('Enter')
  await settle()
  expect(invoked).not.toContain('select_step')
})

test('コアが読めなくても面は開き、理由が出る。移動はできない', async () => {
  const CORE_MISSING = '保存された状態を読み込めていない。'
  disclosureOverride = { stateError: CORE_MISSING, rows: [] }
  await openDisclosure()

  expect(document.querySelector('.disclosure')).not.toBeNull()
  expect(noticeText()).toContain(CORE_MISSING)
  expect(noticeText()).toContain('現在地を移すことはできない')

  press('Enter')
  await settle()
  expect(invoked).not.toContain('select_step')
})

test('一覧を取得できなければ理由を出し、古い行に対して選択を撃たない', async () => {
  await openDisclosure()
  expect(stepRows()).toHaveLength(3)

  disclosureFails = true
  // 現在地が動いたという報せ。開示面が出ている間は一覧も取り直される (AD-3 鮮度規則)。
  await emit('current_position_changed', null)
  await settle()

  // **古い一覧を残さない。** 残せば、コアが確認できなかった行に対して Enter が撃てる。
  expect(stepRows()).toHaveLength(0)
  expect(headings()).toHaveLength(0)
  expect(noticeText()).toContain('一覧を取得できなかった')

  press('Enter')
  await settle()
  expect(invoked).not.toContain('select_step')
})

test('破棄した後に着いた応答は、捨てた一覧を描き直さない', async () => {
  holdDisclosure = true
  await openDisclosure()
  expect(releaseDisclosure).not.toBeNull()

  // 応答が飛んでいる間にオーバーレイを閉じる。面はここで破棄される (FR-19)。
  await emit('tauri://blur', null)
  await settle()

  // 捨てた後に応答が着く。
  releaseDisclosure?.(disclosureFor(null))
  await settle()
  expect(document.querySelector('.disclosure')).toBeNull()

  // **次の呼び出しがそれを一瞬見せてはならない。** 取り直しはまだ飛んでいる (握った
  // ままである) ため、ここで行が出るなら、それは書き戻された古い一覧である。
  await openDisclosure()

  expect(document.querySelector('.disclosure')).not.toBeNull()
  expect(stepRows()).toHaveLength(0)
  expect(headings()).toHaveLength(0)
})

test('選択に失敗したら理由を面に出し、面は閉じない', async () => {
  mockIPC(
    (cmd, args) => {
      invoked.push(cmd)
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> })
      if (cmd === 'select_step') throw new Error('書き込みに失敗した')
      if (cmd === 'get_disclosure_surface') return surfaceFor(args)
      if (cmd === 'get_overlay_snapshot') return snapshot
      return null
    },
    { shouldMockEvents: true },
  )
  await openDisclosure()

  press('Enter')
  await settle()

  expect(document.querySelector('.disclosure')).not.toBeNull()
  expect(noticeText()).toContain('現在地を移せなかった')
  expect(noticeText()).toContain('書き込みに失敗した')
  expect(invoked).not.toContain('hide_overlay')
})

test('作成の面と開示面は同時に現れない — 一覧からは ⌘N が効かない', async () => {
  await openDisclosure()

  press('n', { metaKey: true })
  await settle()

  expect(document.querySelector('input.title')).toBeNull()
  expect(document.querySelector('.disclosure')).not.toBeNull()
})

test('作成の面と開示面は同時に現れない — 作成中は ⌘L が効かない', async () => {
  await openCreation()

  press('l', { metaKey: true })
  await settle()

  expect(document.querySelector('.disclosure')).toBeNull()
  expect(invoked).not.toContain('get_disclosure_surface')
  expect(document.querySelector('input.title')).not.toBeNull()
})

test('切り替えの確定中は ⌘L で一覧に入らない', async () => {
  installHangingIPC('switch_current_position')

  press('Enter')
  await settle()
  expect(invoked).toContain('switch_current_position')

  await openDisclosure()

  // 入れば、失敗したときの理由を読む機会が面ごと消える。
  expect(document.querySelector('.disclosure')).toBeNull()
})

test('選択の確定中は Esc で面を出ない', async () => {
  installHangingIPC('select_step')
  await openDisclosure()

  press('Enter')
  await settle()
  expect(invoked).toContain('select_step')

  press('Escape')
  await settle()

  expect(document.querySelector('.disclosure')).not.toBeNull()
  expect(invoked).not.toContain('hide_overlay')
})

test('一覧が窓に収まらなくても、切り取られずスクロールで到達できる', async () => {
  await openDisclosure()

  const main = document.querySelector('main') as HTMLElement
  const list = document.querySelector('.disclosure') as HTMLElement
  // `main` が唯一のスクロール容器である。**一覧は自分では巻かず、高さも制限しない** —
  // 入れ子の容器が巻き始めると、どちらが動くかが行の位置で変わる。
  expect(getComputedStyle(main).overflowY).toBe('auto')
  // 一覧自身がスクロール容器になっていないこと。`max-height` + `overflow-y: auto` を
  // 足せばここが落ちる — コメントが名指しで警戒している入れ子の容器である。
  const style = getComputedStyle(list)
  expect(['auto', 'scroll']).not.toContain(style.overflowY)
  expect(['auto', 'scroll']).not.toContain(style.overflowX)
  expect(['auto', 'scroll']).not.toContain(style.overflow)
  expect(['none', '']).toContain(style.maxHeight)
  // 最後の行まで DOM にあること。切り取って描かない。
  expect(rowTexts()[2]).toContain('醤油')
  expect(main.contains(list)).toBe(true)
})
