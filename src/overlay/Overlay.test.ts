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
  resting: false,
  waiting: false,
  waitLabel: null,
  waitLengthMinutes: 5,
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
  return { stateError: null, rows, ideas: ideaList }
}

/**
 * 開示面の末尾に並ぶ思いつき (新しい順)。`DisclosureIdea` と 1:1 — **ID と本文だけ**。
 */
let ideaList: { ideaId: string; content: string }[] = []

/**
 * `capture_idea` の応答。Rust 側の `as_idea_content` と同じく、空白だけの本文は拒む。
 * `captureFails` が立っていれば永続化の失敗を模す。
 */
let captureFails = false
/** 置く応答を握り、任意の時点で返すための仕掛け。遅れて着く応答を作る。 */
let holdCapture = false
let releaseCapture: (() => void) | null = null
function captureIdea(args: unknown): null | Promise<null> {
  if (holdCapture) {
    return new Promise((resolve) => {
      releaseCapture = () => resolve(null)
    })
  }
  const request = ((args ?? {}) as { request?: { content?: string } }).request ?? {}
  if (captureFails) throw new Error('状態の書き込みに失敗した')
  if ((request.content ?? '').trim() === '') {
    throw new Error('思いつきが空である。一行書いてから置くこと。')
  }
  return null
}

/**
 * `begin_wait` の応答。Rust 側の `as_wait_minutes` と同じく、正の整数でなければ拒む。
 * 入れたら、コアが確定させた後の状態をスナップショットへ写す。
 */
let beginWaitOutcome = { waiting: true }
function beginWait(args: unknown): { waiting: boolean } {
  const request =
    ((args ?? {}) as { request?: { minutes?: string; label?: string | null } }).request ?? {}
  const minutes = (request.minutes ?? '').trim()
  if (!/^[0-9]+$/.test(minutes) || Number(minutes) === 0) {
    throw new Error('待つ分数は正の整数で書くこと。待機には入っていない。')
  }
  if (beginWaitOutcome.waiting) {
    snapshot = { ...snapshot, waiting: true, waitLabel: request.label ?? null }
  }
  return beginWaitOutcome
}

/** `return_from_wait` の応答。戻ったら待機を畳んだ状態をスナップショットへ写す。 */
let returnOutcome = { returned: true, moved: true }
let returnFails = false
function returnFromWait(): { returned: boolean; moved: boolean } {
  if (returnFails) throw new Error('状態の書き込みに失敗した')
  if (returnOutcome.returned) snapshot = { ...snapshot, waiting: false, waitLabel: null }
  return returnOutcome
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
let endRestOutcome = { resumed: true }
let beginRestOutcome = { rested: true }
let completionOutcome = { taskFinished: false }
/**
 * 修正の面の下書き。**`get_task_draft` の応答である。**
 *
 * 要求された `taskId` を読んで組み立てる — 無視して固定で返すと、どの見出しから
 * 入っても同じタスクが出る実装が緑のまま通る。
 */
let draftFails = false
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
      if (cmd === 'end_rest') return endRestOutcome
      if (cmd === 'begin_rest') return beginRestOutcome
      if (cmd === 'complete_current_step') return completionOutcome
      if (cmd === 'get_task_draft') {
        if (draftFails) throw new Error('常駐プロセスが応答しない')
        return draftFor(args)
      }
      if (cmd === 'edit_task') return null
      if (cmd === 'capture_idea') return captureIdea(args)
      if (cmd === 'begin_wait') return beginWait(args)
      if (cmd === 'return_from_wait') return returnFromWait()
      return null
    },
    { shouldMockEvents: true },
  )
}

/**
 * src-tauri/src/commands/mod.rs の `task_draft_of` と同じ規則で下書きを組み立てる。
 *
 * `taskId` が `null` なら**現在地**のタスクであり、未着手なら相手が無い。
 */
function draftFor(args: unknown): Record<string, unknown> {
  const request = ((args ?? {}) as { request?: { taskId?: string | null } }).request ?? {}
  const wanted = request.taskId ?? taskAtCurrentPosition()
  const task = TASKS.find((candidate) => candidate.taskId === wanted)
  if (!task) return { stateError: null, taskId: null, title: '', steps: [] }
  return {
    stateError: null,
    taskId: task.taskId,
    title: task.title,
    steps: task.steps.map((step) => ({ stepId: step.stepId, content: step.content })),
  }
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
      if (cmd === 'complete_current_step') return completionOutcome
      if (cmd === 'get_task_draft') return draftFor(args)
      if (cmd === 'edit_task') return null
      if (cmd === 'capture_idea') return captureIdea(args)
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

/** 修正の面へ入る。到達には明示的な打鍵が 1 回要る (FR-2 / AD-15)。 */
async function openEdit(): Promise<void> {
  press('e', { metaKey: true })
  await settle()
}

/** 修正の面に並ぶ、既存のステップの入力欄。 */
function stepEditFields(): HTMLInputElement[] {
  return [...document.querySelectorAll<HTMLInputElement>('input.step-edit')]
}

/** ボタンの並びから、表示名で 1 個を引く。 */
function actionButton(label: string): HTMLButtonElement {
  const button = [...document.querySelectorAll<HTMLButtonElement>('.actions button')].find(
    (candidate) => candidate.textContent?.trim() === label,
  )
  if (!button) throw new Error(`「${label}」のボタンが無い`)
  return button
}

/** ボタンが並んでいるか。無い場合を述べるために使う。 */
function actionLabels(): string[] {
  return [...document.querySelectorAll<HTMLButtonElement>('.actions button')].map(
    (button) => button.textContent?.trim() ?? '',
  )
}

/** 既定表示の打鍵の案内の見出しボタン。 */
function shortcutsToggle(): HTMLButtonElement | null {
  return document.querySelector<HTMLButtonElement>('button.shortcuts-toggle')
}

/** 開いた案内の行。閉じていれば空。 */
function shortcutLines(): string[] {
  return Array.from(document.querySelectorAll('.shortcuts-list li')).map(
    (li) => li.textContent ?? '',
  )
}

/** 既定表示の打鍵の案内を (閉じていれば) 開き、その全文を返す。 */
async function shortcutsText(): Promise<string> {
  const toggle = shortcutsToggle()
  if (!toggle) throw new Error('打鍵の案内が無い')
  if (toggle.getAttribute('aria-expanded') !== 'true') {
    toggle.click()
    await settle()
  }
  return document.querySelector('.shortcuts')?.textContent ?? ''
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
  endRestOutcome = { resumed: true }
  beginRestOutcome = { rested: true }
  completionOutcome = { taskFinished: false }
  draftFails = false
  ideaList = []
  captureFails = false
  holdCapture = false
  releaseCapture = null
  beginWaitOutcome = { waiting: true }
  returnOutcome = { returned: true, moved: true }
  returnFails = false
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
  // 移動できても閉じない。移った先を確かめてから閉じるのは利用者である。
  expect(invoked).not.toContain('hide_overlay')
  // 欄は移った先のメモで埋め直されるため、打った本文が記録されたことを述べる。
  expect(noticeText()).toBe('切り替えた。メモを記録し、現在地を次のステップへ移した。')
})

test('提示されたメモに触れずに Enter — 記入として送らない (SM-C3)', async () => {
  // 読み返しただけの切り替え。提示された本文をそのまま送り返すと、記入率が膨らむ。
  press('Enter')
  await settle()

  expect(argsOf('switch_current_position')).toEqual({
    request: { note: null, declareCompletion: false },
  })
  expect(invoked).not.toContain('hide_overlay')
  expect(noticeText()).toBe('切り替えた。現在地を次のステップへ移した。')
})

test('⌘Enter は完了の宣言 — 同じ一連の操作から 1 打鍵で、メモも同時に確定する', async () => {
  typeNote('接続詞を整える途中 / 次は結論')
  await settle()

  press('Enter', { metaKey: true })
  await settle()

  expect(argsOf('complete_current_step')).toEqual({
    request: { note: '接続詞を整える途中 / 次は結論' },
  })
  // **切り替えではない。** 現在地は動かず、次にどこへ着手するかは利用者が選ぶ。
  expect(invoked).not.toContain('switch_current_position')
})

test('完了を宣言したら一覧が開き、オーバーレイは閉じない', async () => {
  press('Enter', { metaKey: true })
  await settle()

  expect(invoked).toContain('get_disclosure_surface')
  expect(invoked).not.toContain('hide_overlay')
  expect(headingTexts()).toEqual(['原稿', '買い物'])
})

test('完了を宣言したことが、一覧の上に述べられる', async () => {
  press('Enter', { metaKey: true })
  await settle()

  // 黙って一覧が開けば、完了が記録されたのか ⌘L を押し間違えただけなのか分からない。
  expect(noticeText()).toContain('完了を宣言した')
})

test('そのタスクが終わったなら、消えることも併せて述べる', async () => {
  completionOutcome = { taskFinished: true }

  press('Enter', { metaKey: true })
  await settle()

  expect(noticeText()).toContain('一覧から消える')
})

test('完了の宣言に失敗したら理由を面に出し、一覧も開かない', async () => {
  mockIPC(
    (cmd) => {
      invoked.push(cmd)
      if (cmd === 'get_overlay_snapshot') return snapshot
      if (cmd === 'complete_current_step') throw new Error('書き込みに失敗した')
      return null
    },
    { shouldMockEvents: true },
  )

  press('Enter', { metaKey: true })
  await settle()

  expect(noticeText()).toContain('状態は変わっていない')
  expect(invoked).not.toContain('get_disclosure_surface')
  expect(invoked).not.toContain('hide_overlay')
})

test('メモを空にしたまま確定しても切り替えは完了する', async () => {
  typeNote('')
  await settle()

  press('Enter')
  await settle()

  expect(argsOf('switch_current_position')).toEqual({
    request: { note: '', declareCompletion: false },
  })
  expect(invoked).not.toContain('hide_overlay')
  expect(noticeText()).toBe('切り替えた。現在地を次のステップへ移した。')
})

test('呼び出しからメモ確定・離脱までが 5 打鍵以内 (FR-7)', async () => {
  // 1 打鍵目はホットキーによる呼び出し。webview からは観測できないため数に含める。
  const HOTKEY_PRESS = 1
  // **実際に窓へ届いた打鍵を数える。** テストが自分で持つ数ではなく、離脱まで何回
  // 押す必要があったかを見る。切り替えは閉じずに移った先を見せるため、離脱は
  // 確認の後の Esc である — 呼び出し・Enter・Esc の 3 打鍵。
  const observed: string[] = []
  const count = (event: KeyboardEvent) => observed.push(event.key)
  window.addEventListener('keydown', count)

  try {
    typeNote('接続詞を整える途中 / 次は結論')
    await settle()

    press('Enter')
    await settle()
    expect(invoked).toContain('switch_current_position')
    expect(invoked).not.toContain('hide_overlay')

    press('Escape')
    await settle()

    expect(invoked).toContain('hide_overlay')
    expect(HOTKEY_PRESS + observed.length).toBeLessThanOrEqual(5)
  } finally {
    window.removeEventListener('keydown', count)
  }
})

// --- 切り替えで閉じない (spec-switch-stays-open) ------------------------------

/** 第 3/全 6 から移った先。第 4 ステップであり、既存のメモを持つ。 */
const FOURTH = {
  ...SNAPSHOT,
  stepContent: '結論の段落を書く',
  stepOrdinal: 4,
  interruptionNote: '主張を一文に絞る途中',
}

test('切り替えは閉じず、移った先の次の一手と位置情報と切り替えた旨を見せる', async () => {
  snapshot = { ...FOURTH }

  press('Enter')
  await settle()

  expect(invoked).toContain('switch_current_position')
  expect(invoked).not.toContain('hide_overlay')
  // 取り直した既定表示が移った先を示す (AD-3 鮮度規則)。
  expect(invoked.indexOf('get_overlay_snapshot')).toBeGreaterThan(
    invoked.indexOf('switch_current_position'),
  )
  const text = document.body.textContent ?? ''
  expect(text).toContain('結論の段落を書く')
  expect(text).toContain('第 4 ステップ / 全 6 ステップ')
  expect(noticeText()).toContain('切り替えた')
})

test('切り替え直後の表示に、離脱したステップの内容は現れない (FR-2)', async () => {
  snapshot = { ...FOURTH }
  typeNote('接続詞を整える途中 / 次は結論')
  await settle()

  press('Enter')
  await settle()

  const text = document.body.textContent ?? ''
  expect(text).not.toContain('3 段落目を書き直す')
  expect(text).not.toContain('第 3 ステップ')
  // 離脱側へ書いたメモも移った先の欄に持ち越さない。
  expect(noteField().value).toBe('主張を一文に絞る途中')
})

test('移った先にメモがあれば入力欄に入り、カーソルは末尾にある (FR-8)', async () => {
  snapshot = { ...FOURTH }

  press('Enter')
  await settle()

  const input = noteField()
  expect(input.value).toBe('主張を一文に絞る途中')
  expect(document.activeElement).toBe(input)
  expect(input.selectionStart).toBe(input.value.length)
  expect(input.selectionEnd).toBe(input.value.length)
})

test('切り替え直後の Esc で閉じ、現在地は動かさない', async () => {
  snapshot = { ...FOURTH }
  press('Enter')
  await settle()
  invoked = []

  press('Escape')
  await settle()

  expect(invoked).toContain('hide_overlay')
  expect(invoked).not.toContain('switch_current_position')
})

test('切り替え直後にもう一度 Enter を押すと、さらに次へ移り、やはり閉じない', async () => {
  snapshot = { ...FOURTH }
  press('Enter')
  await settle()

  snapshot = { ...FOURTH, stepContent: '参考文献を揃える', stepOrdinal: 5, interruptionNote: null }
  press('Enter')
  await settle()

  expect(invoked.filter((cmd) => cmd === 'switch_current_position')).toHaveLength(2)
  expect(invoked).not.toContain('hide_overlay')
  const text = document.body.textContent ?? ''
  expect(text).toContain('参考文献を揃える')
  expect(text).toContain('第 5 ステップ / 全 6 ステップ')
  expect(text).not.toContain('結論の段落を書く')
  // 二度目も提示されたメモに触れていないので、記入として送らない (SM-C3)。
  expect(lastArgsOf('switch_current_position')).toEqual({
    request: { note: null, declareCompletion: false },
  })
  // 切り替えた旨は 1 行だけである。
  expect(
    [...document.querySelectorAll('[role="alert"]')].filter((node) =>
      (node.textContent ?? '').includes('切り替えた'),
    ),
  ).toHaveLength(1)
})

test('切り替えた旨は次回の呼び出しに残らない', async () => {
  snapshot = { ...FOURTH }
  press('Enter')
  await settle()
  expect(noticeText()).toContain('切り替えた')

  await emit('tauri://blur', null)
  await settle()
  await emit('tauri://focus', null)
  await settle()

  expect(document.body.textContent ?? '').not.toContain('切り替えた')
  expect(document.body.textContent ?? '').toContain('結論の段落を書く')
})

test('切り替えの後の取り直しが失敗しても、切り替えた旨は残る', async () => {
  let switchedAlready = false
  mockIPC(
    (cmd, args) => {
      invoked.push(cmd)
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> })
      if (cmd === 'switch_current_position') {
        switchedAlready = true
        return { moved: true }
      }
      if (cmd === 'get_overlay_snapshot') {
        if (switchedAlready) throw new Error('常駐プロセスが応答しない')
        return snapshot
      }
      return null
    },
    { shouldMockEvents: true },
  )

  press('Enter')
  // 再試行 (10 回 × 100ms) を使い切るまで待つ。
  await settle(SNAPSHOT_RETRIES * SNAPSHOT_RETRY_INTERVAL_MS + 200)

  expect(document.querySelector('textarea.note')).toBeNull()
  const text = document.body.textContent ?? ''
  expect(text).toContain('状態を取得できなかった')
  expect(text).toContain('切り替えた。現在地を次のステップへ移した。')
})

test('最終ステップでは切り替えた旨を述べない', async () => {
  switchOutcome = { moved: false }

  press('Enter')
  await settle()

  expect(document.body.textContent ?? '').not.toContain('切り替えた')
  expect(noticeText()).toContain('次のステップが無い')
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

test('最終ステップでも完了の宣言は切り替えを伴わない', async () => {
  // **この経路はもう switch_current_position を通らない。** 最終ステップでも同じであり、
  // 「移動先が無い」という結末そのものが生じない。
  press('Enter', { metaKey: true })
  await settle()

  expect(invoked).toContain('complete_current_step')
  expect(invoked).not.toContain('switch_current_position')
  expect(noticeText()).not.toContain('次のステップが無い')
})

test('最終ステップで提示されたメモを空にして確定したら、消したと述べる', async () => {
  switchOutcome = { moved: false }
  typeNote('')
  await settle()

  press('Enter')
  await settle()

  expect(argsOf('switch_current_position')).toEqual({
    request: { note: '', declareCompletion: false },
  })
  const notice = document.querySelector('[role="alert"]')?.textContent ?? ''
  expect(notice).toContain('メモは消した')
  // **何も確定しなかったとは述べない。** 消えたメモが残っているように読める。
  expect(notice).not.toContain('記録するものは無かった')
})

test('⌘Enter でもメモを空にして確定したら空欄を送る — コアがメモを消す', async () => {
  typeNote('')
  await settle()

  press('Enter', { metaKey: true })
  await settle()

  expect(argsOf('complete_current_step')).toEqual({ request: { note: '' } })
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
  expect(document.body.textContent ?? '').not.toContain('切り替えた')
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
  expect(await shortcutsText()).toContain('⌘Enter — 完了')
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

test('初期表示に作成の面は現れない (FR-2 / AD-15)', async () => {
  expect(document.querySelector('input.title')).toBeNull()
  expect(document.querySelector('textarea.steps')).toBeNull()
  // 到達の手がかりはあるが、面そのものは出ていない。
  expect(actionLabels()).toContain('新規タスク')
  expect(await shortcutsText()).toContain('⌘N — 新規タスク')
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
      ideaId: null,
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
      ideaId: null,
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
      ideaId: null,
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

test('初期表示に開示面は現れない (FR-19 / FR-2 / AD-15)', async () => {
  expect(document.querySelector('.disclosure')).toBeNull()
  expect(stepRows()).toHaveLength(0)
  // 隠れている間に一覧を取りに行かない。
  expect(invoked).not.toContain('get_disclosure_surface')
  // 到達の手がかりはあるが、面そのものは出ていない。
  expect(actionLabels()).toContain('タスク一覧')
  expect(await shortcutsText()).toContain('⌘L — タスク一覧')
  expect(document.querySelector('.disclosure')).toBeNull()
  expect(invoked).not.toContain('get_disclosure_surface')
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

test('j / k でも ↓ / ↑ と同じく行の間を動く', async () => {
  await openDisclosure()
  const at = (): string | undefined => (document.activeElement as HTMLElement)?.dataset.rowKey

  press('k')
  press('k')
  press('k')
  await settle()
  expect(at()).toBe(`task:${SHOPPING}`)

  press('k')
  await settle()
  expect(at()).toBe(`task:${MANUSCRIPT}`)

  const down = pressCancelable('j')
  await settle()
  expect(at()).toBe(`task:${SHOPPING}`)
  // 打鍵は飲む (webview の既定動作へ抜けさせない)。
  expect(down.defaultPrevented).toBe(true)

  // CapsLock の J / K も同じ打鍵である。
  press('K')
  await settle()
  expect(at()).toBe(`task:${MANUSCRIPT}`)
})

test('修飾キーを伴う j / k は移動に読まない', async () => {
  await openDisclosure()
  const before = (document.activeElement as HTMLElement)?.dataset.rowKey

  press('k', { metaKey: true })
  press('k', { ctrlKey: true })
  press('k', { altKey: true })
  press('k', { shiftKey: true })
  await settle()

  expect((document.activeElement as HTMLElement)?.dataset.rowKey).toBe(before)
})

test('一覧の案内に j k が載る', async () => {
  await openDisclosure()

  expect(document.querySelector('.hint')?.textContent ?? '').toContain('↑↓ (j k) で移動')
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
  disclosureOverride = { stateError: null, rows: [], ideas: [] }
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
  disclosureOverride = { stateError: CORE_MISSING, rows: [], ideas: [] }
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


// --- 休息中の既定表示 (CAP-10 / FR-15) --------------------------------------

/**
 * 休息中の状態でオーバーレイを描き直す。
 *
 * **スナップショットを差し替えてから取り直す。** フロントは計時をしない (AD-8) ため、
 * 「休息中である」はコアから届く事実そのものである。
 */
async function enterTheRestingSurface(): Promise<void> {
  snapshot = { ...SNAPSHOT, resting: true }
  await emit('tauri://focus', null)
  await settle()
  invoked = []
  calls = []
}

test('休息中は次の一手を出さない — 現在地が保たれていることだけを述べる', async () => {
  await enterTheRestingSurface()

  const text = document.body.textContent ?? ''
  expect(text).toContain('休息中')
  expect(text).not.toContain(SNAPSHOT.stepContent)
  expect(document.querySelector('textarea.note')).toBeNull()
})

test('受け入れ条件: 休息中に Enter を押すと休息の終了が宣言される', async () => {
  await enterTheRestingSurface()

  press('Enter')
  await settle()

  expect(invoked).toContain('end_rest')
  // **切り替えではない。** 切り替えれば現在地が黙って活性へ戻る。
  expect(invoked).not.toContain('switch_current_position')
})

test('休息の終了の後、既定表示は取り直されて CAP-8 の形式に戻る', async () => {
  await enterTheRestingSurface()
  // 宣言の応答を境に、コアは活性の現在地を返す。
  snapshot = { ...SNAPSHOT, resting: false }

  press('Enter')
  await settle()

  expect(invoked).toContain('get_overlay_snapshot')
  const text = document.body.textContent ?? ''
  expect(text).toContain(SNAPSHOT.stepContent)
  expect(text).toContain('第 3 ステップ / 全 6 ステップ')
  // 中断メモは入力欄の初期値として戻る (FR-7 / FR-8)。
  expect(noteField().value).toBe(SNAPSHOT.interruptionNote)
})

test('休息中の ⌘Enter も切り替えを撃たない', async () => {
  await enterTheRestingSurface()

  press('Enter', { metaKey: true })
  await settle()

  expect(invoked).not.toContain('switch_current_position')
})

test('休息中ではなかったと返ったら、そのことを述べる', async () => {
  await enterTheRestingSurface()
  endRestOutcome = { resumed: false }
  snapshot = { ...SNAPSHOT, resting: false }

  press('Enter')
  await settle()

  expect(noticeText()).toContain('休息中ではなかった')
})

test('休息の終了に失敗したら理由を出し、面は閉じない', async () => {
  await enterTheRestingSurface()
  mockIPC(
    (cmd, args) => {
      invoked.push(cmd)
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> })
      if (cmd === 'end_rest') throw new Error('書き込めない')
      if (cmd === 'get_overlay_snapshot') return snapshot
      return null
    },
    { shouldMockEvents: true },
  )

  press('Enter')
  await settle()

  expect(noticeText()).toContain('書き込めない')
  expect(invoked).not.toContain('hide_overlay')
})

test('休息中は案内が入れ替わる — 切り替えも完了の宣言も案内しない', async () => {
  await enterTheRestingSurface()

  const hint = await shortcutsText()
  expect(hint).toContain('Enter — 休息を終える')
  expect(hint).not.toContain('切り替え')
  expect(hint).not.toContain('完了')
})

test('休息中でも残り時間やカウントダウンを出さない (spec Never)', async () => {
  await enterTheRestingSurface()

  const text = document.body.textContent ?? ''
  for (const forbidden of ['残り', '分後', '経過', '%']) {
    expect(text).not.toContain(forbidden)
  }
})

// --- 休息のすすめ ----------------------------------------------------------

function restAdviceToggle(): HTMLButtonElement | null {
  return document.querySelector<HTMLButtonElement>('button.rest-advice-toggle')
}

function restAdviceItems(): string[] {
  return Array.from(document.querySelectorAll('.rest-advice-list li')).map(
    (li) => li.textContent ?? '',
  )
}

test('休息に入った直後のすすめは見出しだけである (閉じた状態)', async () => {
  await enterTheRestingSurface()

  const toggle = restAdviceToggle()
  expect(toggle?.textContent).toContain('休息のすすめ')
  expect(toggle?.getAttribute('aria-expanded')).toBe('false')
  expect(restAdviceItems()).toHaveLength(0)
})

test('⌘T ですすめが開き、もう一度で畳まれる — コアは呼ばれない', async () => {
  await enterTheRestingSurface()

  const opened = pressCancelable('t', { metaKey: true })
  await settle()
  expect(opened.defaultPrevented).toBe(true)
  expect(restAdviceToggle()?.getAttribute('aria-expanded')).toBe('true')
  expect(restAdviceItems()).toEqual([
    '窓の外など遠く (6m 以上) を 20 秒ほど眺める',
    '水をコップ 1 杯飲む',
    'スマホを見ない — うつむく姿勢が首に、近くの画面が目に負担をかける',
    '肩をすくめて 5 秒止め、ストンと落とす (3 回)',
    '首をゆっくり左右に倒し、伸ばす',
    '歯の食いしばりをほどき、ゆっくり息を吐く',
    '立ち上がって少し歩く',
  ])

  // CapsLock が入っていても効く。
  press('T', { metaKey: true })
  await settle()
  expect(restAdviceToggle()?.getAttribute('aria-expanded')).toBe('false')
  expect(restAdviceItems()).toHaveLength(0)

  expect(invoked).toEqual([])
})

test('ボタンでもすすめを開閉でき、コアは呼ばれない', async () => {
  await enterTheRestingSurface()

  restAdviceToggle()?.click()
  await settle()
  expect(restAdviceToggle()?.getAttribute('aria-expanded')).toBe('true')
  expect(restAdviceItems()).toHaveLength(7)

  restAdviceToggle()?.click()
  await settle()
  expect(restAdviceToggle()?.getAttribute('aria-expanded')).toBe('false')
  expect(restAdviceItems()).toHaveLength(0)

  expect(invoked).toEqual([])
})

test('修飾を伴う ⌘T は開閉しない', async () => {
  await enterTheRestingSurface()

  press('t', { metaKey: true, shiftKey: true })
  press('t', { metaKey: true, altKey: true })
  press('t', { metaKey: true, ctrlKey: true })
  press('t')
  await settle()

  expect(restAdviceToggle()?.getAttribute('aria-expanded')).toBe('false')
})

test('IME 変換中の ⌘T は何もしない', async () => {
  await enterTheRestingSurface()

  press('t', { metaKey: true, isComposing: true })
  await settle()

  expect(restAdviceToggle()?.getAttribute('aria-expanded')).toBe('false')
  expect(invoked).toEqual([])
})

test('作業中の既定表示にすすめは描かれず、⌘T は何も起こさない', async () => {
  expect(restAdviceToggle()).toBeNull()
  invoked = []

  const event = pressCancelable('t', { metaKey: true })
  await settle()

  expect(event.defaultPrevented).toBe(true)
  expect(restAdviceToggle()).toBeNull()
  expect(document.body.textContent ?? '').not.toContain('休息のすすめ')
  expect(invoked).toEqual([])
  expect(await shortcutsText()).not.toContain('⌘T')

  // 作業中の ⌘T は引き継がれる開閉の状態にも触れない。
  await enterTheRestingSurface()
  expect(restAdviceToggle()?.getAttribute('aria-expanded')).toBe('false')
  expect(restAdviceItems()).toHaveLength(0)
})

test('押しっぱなしの ⌘T (自動反復) は開閉しない', async () => {
  await enterTheRestingSurface()

  const event = pressCancelable('t', { metaKey: true, repeat: true })
  await settle()

  expect(event.defaultPrevented).toBe(true)
  expect(restAdviceToggle()?.getAttribute('aria-expanded')).toBe('false')
  expect(restAdviceItems()).toHaveLength(0)
})

test('休息中の案内に ⌘T が載る', async () => {
  await enterTheRestingSurface()

  expect(await shortcutsText()).toContain('⌘T — 休息のすすめ')
})

test('すすめが開いたまま Enter を押すと休息が終わり、すすめは消える', async () => {
  await enterTheRestingSurface()
  press('t', { metaKey: true })
  await settle()
  snapshot = { ...SNAPSHOT, resting: false }

  press('Enter')
  await settle()

  expect(invoked).toContain('end_rest')
  expect(invoked).not.toContain('switch_current_position')
  expect(restAdviceToggle()).toBeNull()
  expect(restAdviceItems()).toHaveLength(0)
})

test('開閉の状態は閉じても次の休息へ引き継がれる', async () => {
  await enterTheRestingSurface()
  press('t', { metaKey: true })
  await settle()

  // 休息を終え、オーバーレイを閉じる。
  snapshot = { ...SNAPSHOT, resting: false }
  press('Enter')
  await settle()
  press('Escape')
  await settle()
  expect(invoked).toContain('hide_overlay')

  // 後で再び休息に入る。
  await enterTheRestingSurface()

  expect(restAdviceToggle()?.getAttribute('aria-expanded')).toBe('true')
  expect(restAdviceItems()).toHaveLength(7)
})

test('すすめを開いても残り時間・経過・達成を出さない (AD-15)', async () => {
  await enterTheRestingSurface()
  press('t', { metaKey: true })
  await settle()

  const text = document.body.textContent ?? ''
  for (const forbidden of ['残り', '分後', '経過', '%']) {
    expect(text).not.toContain(forbidden)
  }
  expect(document.querySelector('.rest-advice input[type="checkbox"]')).toBeNull()
})

test('受け入れ条件: 開いたすすめは切り取られず、main のスクロールで辿れる', async () => {
  await enterTheRestingSurface()
  press('t', { metaKey: true })
  await settle()

  const main = document.querySelector('main') as HTMLElement
  const list = document.querySelector('.rest-advice-list') as HTMLElement
  expect(main.contains(list)).toBe(true)
  const style = getComputedStyle(list)
  expect(['auto', 'scroll', 'hidden']).not.toContain(style.overflow)
  expect(['auto', 'scroll', 'hidden']).not.toContain(style.overflowY)
  expect(['auto', 'scroll', 'hidden']).not.toContain(style.overflowX)
  expect(['none', '']).toContain(style.maxHeight)
  expect(['', 'auto']).toContain(style.height)
  expect(restAdviceItems()[6]).toContain('立ち上がって少し歩く')
})

// ---------------------------------------------------------------------------
// 修正の面 (CAP-5 / FR-4 / FR-5)
// ---------------------------------------------------------------------------

test('初期表示に修正の面は現れない (FR-2 / AD-15)', () => {
  expect(document.querySelector('input.title')).toBeNull()
  expect(stepEditFields()).toHaveLength(0)
})

test('⌘E で修正の面に入り、現在地のタスクが下書きになる', async () => {
  await openEdit()

  expect(argsOf('get_task_draft')).toEqual({ request: { taskId: null } })
  expect(titleField().value).toBe('買い物')
  expect(stepEditFields().map((field) => field.value)).toEqual(['米', '味噌', '醤油'])
  // 入力位置は題名の欄にある (I/O マトリクス「面へ入る」)。
  expect(document.activeElement).toBe(titleField())
})

test('修正の面は一度に一つのタスクしか出さない', async () => {
  await openEdit()

  const text = document.body.textContent ?? ''
  expect(text).not.toContain('原稿')
  expect(text).not.toContain('構成を決める')
})

test('⌘Enter で題名・本文・追記が一度に送られる — ID と本文の対で', async () => {
  await openEdit()
  typeInto(titleField(), '買い出し')
  typeInto(stepEditFields()[1], '味噌 (白)')
  typeInto(stepsField(), '酢\n\n みりん ')
  await settle()

  press('Enter', { metaKey: true })
  await settle()

  expect(argsOf('edit_task')).toEqual({
    request: {
      taskId: SHOPPING,
      title: '買い出し',
      steps: [
        { stepId: '0198f0e0-0000-7000-8000-000000000003', content: '米' },
        { stepId: '0198f0e0-0000-7000-8000-000000000004', content: '味噌 (白)' },
        { stepId: '0198f0e0-0000-7000-8000-000000000005', content: '醤油' },
      ],
      // **切り分けはコマンド境界が持つ。** 空行も前後の空白もそのまま送る。
      addedSteps: '酢\n\n みりん ',
    },
  })
})

test('保存に成功すると面は畳まれ、直したことが述べられる', async () => {
  await openEdit()

  press('Enter', { metaKey: true })
  await settle()

  expect(stepEditFields()).toHaveLength(0)
  expect(noticeText()).toContain('タスクを直した')
})

test('保存に失敗したら面は閉じず、入力も理由も残る', async () => {
  await openEdit()
  typeInto(titleField(), '   ')
  await settle()

  mockIPC(
    (cmd, args) => {
      invoked.push(cmd)
      if (cmd === 'get_overlay_snapshot') return snapshot
      if (cmd === 'get_task_draft') return draftFor(args)
      if (cmd === 'edit_task') throw new Error('題名が空である。')
      return null
    },
    { shouldMockEvents: true },
  )

  press('Enter', { metaKey: true })
  await settle()

  expect(titleField().value).toBe('   ')
  expect(noticeText()).toContain('何も保存されていない')
})

test('Esc は下書きを捨てて既定表示へ戻る — オーバーレイは閉じない', async () => {
  await openEdit()
  typeInto(titleField(), '打ち間違い')
  await settle()

  press('Escape')
  await settle()

  expect(stepEditFields()).toHaveLength(0)
  expect(invoked).not.toContain('hide_overlay')
  expect(invoked).not.toContain('edit_task')
})

test('修正の面では素の Enter は改行であって保存ではない', async () => {
  await openEdit()

  press('Enter')
  press('Enter', { shiftKey: true })
  await settle()

  expect(invoked).not.toContain('edit_task')
})

test('⌘⇧Enter はどこにも束縛されていない — 直したついでに現在地が動かない', async () => {
  await openEdit()

  press('Enter', { metaKey: true, shiftKey: true })
  await settle()

  expect(invoked).not.toContain('edit_task')
  expect(invoked).not.toContain('select_step')
})

test('フォーカスを失うと下書きは失われ、修正の面も畳まれる', async () => {
  await openEdit()
  typeInto(titleField(), '打ち間違い')
  await settle()

  await emit('tauri://blur', null)
  await settle()
  await emit('tauri://focus', null)
  await settle()

  expect(stepEditFields()).toHaveLength(0)
  expect(invoked).not.toContain('edit_task')
})

test('未着手では ⌘E が何も起こさない — 直す相手が無い', async () => {
  snapshot = { ...SNAPSHOT, stepContent: null, stepOrdinal: null, stepCount: null }
  currentStepId = null
  if (component) unmount(component)
  await mountOverlay()
  invoked = []

  await openEdit()

  expect(invoked).not.toContain('get_task_draft')
})

test('一覧からは選んでいる行のタスクを直せる — 現在地は動かない', async () => {
  await openDisclosure()
  await moveSelectionTo(`task:${MANUSCRIPT}`)
  invoked = []

  press('e', { metaKey: true })
  await settle()

  expect(argsOf('get_task_draft')).toEqual({ request: { taskId: MANUSCRIPT } })
  expect(titleField().value).toBe('原稿')
  expect(invoked).not.toContain('select_step')
})

test('修正の面は進捗率も件数も完了の印も描かない (AD-15)', async () => {
  await openEdit()

  const text = document.body.textContent ?? ''
  for (const forbidden of ['%', '件', '✓', '全 3']) {
    expect(text).not.toContain(forbidden)
  }
  expect(document.querySelector('progress')).toBeNull()
})

// ---------------------------------------------------------------------------
// マウス操作 (ボタン)
// ---------------------------------------------------------------------------

test('既定表示の操作はボタンからも到達できる', async () => {
  expect(actionLabels()).toEqual([
    '完了',
    '切り替え',
    '思いつき',
    '待機',
    '新規タスク',
    'タスク修正',
    'タスク一覧',
    '休息',
    '閉じる',
  ])
})

/** 作業中の既定表示のボタンを段ごとに読む。 */
function actionRows(): string[][] {
  return [...document.querySelectorAll('.actions .action-row')].map((row) =>
    [...row.querySelectorAll('button')].map((button) => button.textContent?.trim() ?? ''),
  )
}

test('I/O「作業中」— ボタンは三つのまとまりに分かれる', () => {
  expect(actionRows()).toEqual([
    ['完了', '切り替え', '思いつき', '待機'],
    ['新規タスク', 'タスク修正', 'タスク一覧'],
    ['休息', '閉じる'],
  ])
})

test('受け入れ条件: 前の二つのまとまりは横に並び、休息と閉じるはその下に置かれる', () => {
  const actions = document.querySelector('.actions') as HTMLElement
  const line = actions.querySelector(':scope > .action-line') as HTMLElement
  expect(getComputedStyle(actions).flexDirection).toBe('column')
  // 1 段目と 2 段目は同じ行の中にある (利用者の裁定 — 縦に三段は読みにくかった)。
  expect(getComputedStyle(line).display).toBe('flex')
  expect(getComputedStyle(line).flexDirection).toBe('row')
  // 窓の幅が足りなければ 2 段目は下へ折り返す — 切り取らない。
  expect(getComputedStyle(line).flexWrap).toBe('wrap')
  expect(
    [...line.querySelectorAll(':scope > .action-row')].map(
      (row) => row.querySelector('button')?.textContent?.trim(),
    ),
  ).toEqual(['完了', '新規タスク'])
  // 休息と閉じるは、その行の下に独立して置かれる。
  const below = actions.querySelectorAll(':scope > .action-row')
  expect(below).toHaveLength(1)
  expect(below[0].textContent).toContain('休息')
  // 隙間の大小は jsdom が計算しない (gap を返さない)。make dev で目で見る。
  // まとまりの外にボタンを置かない (まとまりに属さないボタンを作らない)。
  expect(actions.querySelectorAll(':scope > button')).toHaveLength(0)
  expect(line.querySelectorAll(':scope > button')).toHaveLength(0)
})

test('I/O「待機中」— 1 段目の 4 つ目が「主作業に戻る」に入れ替わる', async () => {
  snapshot = { ...SNAPSHOT, waiting: true, waitLabel: null }
  if (component) unmount(component)
  await mountOverlay()

  expect(actionRows()[0]).toEqual(['完了', '切り替え', '思いつき', '主作業に戻る'])
  expect(await shortcutsText()).toContain('⌘D — 主作業に戻る')
})

test('I/O「思いつきボタン」— ⌘I と同じ一行の面が開く', async () => {
  actionButton('思いつき').click()
  await settle()

  expect(document.querySelector('input.idea')).not.toBeNull()
  expect(document.activeElement).toBe(document.querySelector('input.idea'))
  expect(invoked).not.toContain('capture_idea')
})

// --- 打鍵の案内 (折りたたみ) ------------------------------------------------

test('I/O「案内 (初回)」— 起動後はじめて開くと見出し 1 行だけである', () => {
  expect(shortcutsToggle()?.getAttribute('aria-expanded')).toBe('false')
  expect(shortcutsToggle()?.textContent).toContain('▸')
  expect((document.querySelector('.shortcuts')?.textContent ?? '').replace(/\s+/g, '')).toBe('▸ショートカット')
  expect(shortcutLines()).toHaveLength(0)
  expect(document.querySelector('.shortcuts')?.textContent ?? '').not.toContain('⌘')
})

test('I/O「案内を開く」— ⌘/ で打鍵ごとに 1 行で並び、もう一度で畳む', async () => {
  invoked = []
  const event = pressCancelable('/', { metaKey: true })
  await settle()

  expect(event.defaultPrevented).toBe(true)
  expect(shortcutsToggle()?.getAttribute('aria-expanded')).toBe('true')
  expect(shortcutsToggle()?.textContent).toContain('▾')
  expect(shortcutLines()).toEqual([
    'Enter — 切り替え',
    '⌘Enter — 完了',
    '⌘D — 待機',
    '⌘E — タスク修正',
    '⌘R — 休息',
    '⌘N — 新規タスク',
    '⌘I — 思いつき',
    '⌘L — タスク一覧',
    '⌘/ — ショートカットの開閉',
    'Esc — 閉じる',
    `${SNAPSHOT.hotkey.accelerator} — オーバーレイの開閉`,
    '開く・終了はメニューバー項目からも',
  ])
  // 開閉はコアを呼ばない。
  expect(invoked).toEqual([])

  press('/', { metaKey: true })
  await settle()
  expect(shortcutsToggle()?.getAttribute('aria-expanded')).toBe('false')
  expect(shortcutLines()).toHaveLength(0)
})

test('受け入れ条件: 開いた案内は 1 行に打鍵が一つだけ載っている', async () => {
  await shortcutsText()

  const accelerator = SNAPSHOT.hotkey.accelerator.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const keys = new RegExp(`(⌘?Enter|Esc|⌘[A-Z/]|${accelerator})`, 'g')
  const lines = shortcutLines()
  expect(lines.some((line) => line.includes(SNAPSHOT.hotkey.accelerator))).toBe(true)
  for (const line of lines) {
    expect((line.match(keys) ?? []).length).toBeLessThanOrEqual(1)
  }
})

test('作業中に開いた案内は、休息に入っても戻っても開いたままである', async () => {
  press('/', { metaKey: true })
  await settle()

  await enterTheRestingSurface()
  expect(shortcutsToggle()?.getAttribute('aria-expanded')).toBe('true')
  expect(shortcutLines()).toContain('Enter — 休息を終える')

  snapshot = { ...SNAPSHOT, resting: false }
  press('Enter')
  await settle()
  expect(invoked).toContain('end_rest')
  expect(shortcutsToggle()?.getAttribute('aria-expanded')).toBe('true')
  expect(shortcutLines()).toContain('Enter — 切り替え')
})

test('思いつきのボタンは切り替えの確定中には面を開かない (⌘I と同じ)', async () => {
  installHangingIPC('switch_current_position')
  press('Enter')
  await settle()
  expect(invoked).toContain('switch_current_position')

  actionButton('思いつき').click()
  await settle()

  expect(document.querySelector('input.idea')).toBeNull()
})

test('I/O「案内を開く」— 見出しのクリックでも開閉する', async () => {
  shortcutsToggle()?.click()
  await settle()
  expect(shortcutsToggle()?.getAttribute('aria-expanded')).toBe('true')
  expect(shortcutsToggle()?.getAttribute('aria-controls')).toBe('shortcuts-list')

  shortcutsToggle()?.click()
  await settle()
  expect(shortcutsToggle()?.getAttribute('aria-expanded')).toBe('false')
  expect(shortcutsToggle()?.hasAttribute('aria-controls')).toBe(false)
})

test('I/O「案内の記憶」— 開いたまま閉じて、また開くと開いた状態で出る', async () => {
  press('/', { metaKey: true })
  await settle()
  press('Escape')
  await settle()
  expect(invoked).toContain('hide_overlay')

  await emit('tauri://focus', null)
  await settle()

  expect(shortcutsToggle()?.getAttribute('aria-expanded')).toBe('true')
  expect(shortcutLines().length).toBeGreaterThan(0)
})

test('押しっぱなしの ⌘/ (自動反復) は開閉しない', async () => {
  const event = pressCancelable('/', { metaKey: true, repeat: true })
  await settle()

  expect(event.defaultPrevented).toBe(true)
  expect(shortcutsToggle()?.getAttribute('aria-expanded')).toBe('false')
})

test('I/O「休息中の案内」— ⌘/ で休息中の打鍵が並び、ボタンは今のまま', async () => {
  await enterTheRestingSurface()
  press('/', { metaKey: true })
  await settle()

  const lines = shortcutLines()
  expect(lines).toContain('Enter — 休息を終える')
  expect(lines).toContain('⌘T — 休息のすすめ')
  expect(lines.join('\n')).not.toContain('切り替え')
  expect(lines.join('\n')).not.toContain('⌘R')
  expect(lines.join('\n')).not.toContain('⌘E')
  expect(actionLabels()).toEqual(['休息を終える', '新規タスク', 'タスク一覧', '閉じる'])
  expect(invoked).toEqual([])
})

test('未着手の案内は現在地に掛かる打鍵を載せない', async () => {
  snapshot = { ...SNAPSHOT, stepContent: null, stepOrdinal: null, stepCount: null }
  if (component) unmount(component)
  await mountOverlay()

  expect(actionLabels()).toEqual(['新規タスク', 'タスク一覧', '閉じる'])
  const text = await shortcutsText()
  for (const absent of ['Enter — 切り替え', '⌘Enter', '⌘D', '⌘E', '⌘R', '⌘T']) {
    expect(text).not.toContain(absent)
  }
  expect(text).toContain('⌘N — 新規タスク')
})

test('ホットキーが登録できていなければ、その行を案内に載せない', async () => {
  snapshot = {
    ...SNAPSHOT,
    hotkey: { accelerator: 'Control + Option + Space', registered: false, error: '衝突' },
  }
  if (component) unmount(component)
  await mountOverlay()

  const text = await shortcutsText()
  expect(text).not.toContain('Control + Option + Space — オーバーレイの開閉')
  expect(text).toContain('開く・終了はメニューバー項目からも')
})

test('I/O「他の面」— 作成の面で ⌘/ は何も起こさず、1 行の案内のまま', async () => {
  await openCreation()
  press('/', { metaKey: true })
  await settle()

  expect(shortcutsToggle()).toBeNull()
  expect(document.querySelector('.hint')?.textContent ?? '').toContain('⌘Enter で作成')

  press('Escape')
  await settle()
  expect(shortcutsToggle()?.getAttribute('aria-expanded')).toBe('false')
})

test('I/O「IME」— 変換中の ⌘/ は何もしない', async () => {
  press('/', { metaKey: true, isComposing: true } as KeyboardEventInit)
  await settle()

  expect(shortcutsToggle()?.getAttribute('aria-expanded')).toBe('false')
})

test('「切り替え」のボタンは Enter と同じ経路を通る', async () => {
  typeNote('接続詞を整える途中 / 次は結論')
  await settle()

  actionButton('切り替え').click()
  await settle()

  expect(argsOf('switch_current_position')).toEqual({
    request: { note: '接続詞を整える途中 / 次は結論', declareCompletion: false },
  })
  // Enter と同じく閉じずに移った先を見せる。
  expect(invoked).not.toContain('hide_overlay')
  expect(noticeText()).toContain('切り替えた')
})

test('「完了」のボタンは ⌘Enter と同じ経路を通る', async () => {
  actionButton('完了').click()
  await settle()

  expect(invoked).toContain('complete_current_step')
  expect(invoked).toContain('get_disclosure_surface')
})

test('ボタンの押下は入力位置を奪わない', async () => {
  const note = noteField()
  note.focus()

  // 既定動作 (フォーカスの移動) が止められていること。
  const event = new MouseEvent('mousedown', { bubbles: true, cancelable: true })
  actionButton('タスク一覧').dispatchEvent(event)

  expect(event.defaultPrevented).toBe(true)
  expect(document.activeElement).toBe(note)
})

test('一覧の確定もボタンから到達でき、案内は行の種類で変わる', async () => {
  await openDisclosure()

  expect(actionLabels()).toContain('ここへ現在地を移す')
  actionButton('ここへ現在地を移す').click()
  await settle()

  expect(invoked).toContain('select_step')
})

test('一覧の見出しの上ではボタンの文言も入れ替わる', async () => {
  await openDisclosure()
  await moveSelectionTo(`task:${MANUSCRIPT}`)

  expect(actionLabels()).toContain('このタスクのステップを見る')
  expect(actionLabels()).not.toContain('ここへ現在地を移す')
})

test('休息中のボタンは休息の終了だけである', async () => {
  await enterTheRestingSurface()

  expect(actionLabels()).toContain('休息を終える')
  expect(actionLabels()).not.toContain('切り替え')
  expect(actionLabels()).not.toContain('完了')

  actionButton('休息を終える').click()
  await settle()
  expect(invoked).toContain('end_rest')
})

test('作成の面のボタンは二つの確定と戻るだけである', async () => {
  await openCreation()

  expect(actionLabels()).toEqual(['作成', '作成して着手', '戻る'])

  typeInto(titleField(), '原稿')
  typeInto(stepsField(), '下書き')
  await settle()
  actionButton('作成して着手').click()
  await settle()

  expect(argsOf('create_task')).toEqual({
    request: { title: '原稿', steps: '下書き', moveCurrentPosition: true, ideaId: null },
  })
})

test('「新規タスク」のボタンは ⌘N と同じ経路を通る', async () => {
  actionButton('新規タスク').click()
  await settle()

  expect(document.querySelector('input.title')).not.toBeNull()
  expect(document.activeElement).toBe(titleField())
})

test('「タスク修正」のボタンは ⌘E と同じ経路を通る', async () => {
  actionButton('タスク修正').click()
  await settle()

  expect(invoked).toContain('get_task_draft')
})

test('「タスク一覧」のボタンは ⌘L と同じ経路を通る', async () => {
  actionButton('タスク一覧').click()
  await settle()

  expect(invoked).toContain('get_disclosure_surface')
})

test('休息中の「新規タスク」「タスク一覧」「閉じる」もそれぞれの経路を通る', async () => {
  await enterTheRestingSurface()
  actionButton('新規タスク').click()
  await settle()
  expect(document.querySelector('input.title')).not.toBeNull()
  press('Escape')
  await settle()

  invoked = []
  actionButton('タスク一覧').click()
  await settle()
  expect(invoked).toContain('get_disclosure_surface')
  press('Escape')
  await settle()

  invoked = []
  actionButton('閉じる').click()
  await settle()
  expect(invoked).toContain('hide_overlay')
})

test('「閉じる」のボタンは Esc と同じ経路を通る', async () => {
  actionButton('閉じる').click()
  await settle()

  expect(invoked).toContain('hide_overlay')
})

// ---------------------------------------------------------------------------
// 休息への手動の切り替え (CAP-10 / FR-15)
// ---------------------------------------------------------------------------

test('⌘R で休息に入る — 介入を待たない', async () => {
  press('r', { metaKey: true })
  await settle()

  expect(invoked).toContain('begin_rest')
  // 面は取り直され、オーバーレイは閉じない。
  expect(invoked).toContain('get_overlay_snapshot')
  expect(invoked).not.toContain('hide_overlay')
})

test('休息に入ると面が「休息中」へ入れ替わる', async () => {
  // コアが確定させた後の状態を返す。取り直しがそれを描く。
  mockIPC(
    (cmd) => {
      invoked.push(cmd)
      if (cmd === 'begin_rest') {
        snapshot = { ...SNAPSHOT, resting: true }
        return beginRestOutcome
      }
      if (cmd === 'get_overlay_snapshot') return snapshot
      return null
    },
    { shouldMockEvents: true },
  )

  press('r', { metaKey: true })
  await settle()

  const text = document.body.textContent ?? ''
  expect(text).toContain('休息中')
  expect(text).not.toContain(SNAPSHOT.stepContent)
  // 案内も入れ替わる。
  expect(await shortcutsText()).toContain('Enter — 休息を終える')
})

test('「休息」のボタンは ⌘R と同じ経路を通る', async () => {
  actionButton('休息').click()
  await settle()

  expect(invoked).toContain('begin_rest')
})

test('休息中には ⌘R もボタンも無い — Enter は終了の宣言だけを意味する', async () => {
  await enterTheRestingSurface()

  expect(actionLabels()).not.toContain('休息')
  expect(await shortcutsText()).not.toContain('⌘R')

  press('r', { metaKey: true })
  await settle()
  expect(invoked).not.toContain('begin_rest')
})

test('未着手では休息に入れない — 止める計数が無い', async () => {
  snapshot = { ...SNAPSHOT, stepContent: null, stepOrdinal: null, stepCount: null }
  if (component) unmount(component)
  await mountOverlay()
  invoked = []

  press('r', { metaKey: true })
  await settle()

  expect(invoked).not.toContain('begin_rest')
  expect(actionLabels()).not.toContain('休息')
})

test('何も書かれなかったなら、そう述べる', async () => {
  beginRestOutcome = { rested: false }

  press('r', { metaKey: true })
  await settle()

  expect(noticeText()).toContain('休息に入れなかった')
})

test('宣言に失敗したら理由を面に出し、閉じない', async () => {
  mockIPC(
    (cmd) => {
      invoked.push(cmd)
      if (cmd === 'get_overlay_snapshot') return snapshot
      if (cmd === 'begin_rest') throw new Error('書き込みに失敗した')
      return null
    },
    { shouldMockEvents: true },
  )

  press('r', { metaKey: true })
  await settle()

  expect(noticeText()).toContain('状態は変わっていない')
  expect(invoked).not.toContain('hide_overlay')
})

test('作成の面と一覧では ⌘R が休息を宣言しない', async () => {
  await openCreation()
  press('r', { metaKey: true })
  await settle()
  expect(invoked).not.toContain('begin_rest')

  press('Escape')
  await settle()
  await openDisclosure()
  press('r', { metaKey: true })
  await settle()
  expect(invoked).not.toContain('begin_rest')
})

// ---------------------------------------------------------------------------
// 思いつき — 作業中の思いつきを一行で手放す
// ---------------------------------------------------------------------------

const IDEA_A = { ideaId: '0198f0e0-0000-7000-8000-0000000000c1', content: '請求書の様式を直す' }
const IDEA_B = { ideaId: '0198f0e0-0000-7000-8000-0000000000c2', content: '棚を片付ける' }

function ideaField(): HTMLInputElement | null {
  return document.querySelector<HTMLInputElement>('input.idea')
}

async function openCapture(): Promise<void> {
  press('i', { metaKey: true })
  await settle()
}

function ideaRows(): HTMLButtonElement[] {
  return [...document.querySelectorAll<HTMLButtonElement>('.idea-row')]
}

test('思いつきの面は初期表示に無く、⌘I で一行の入力欄が開く', async () => {
  expect(ideaField()).toBeNull()

  await openCapture()

  expect(ideaField()).not.toBeNull()
  expect(document.activeElement).toBe(ideaField())
  expect(document.querySelector('.hint')?.textContent ?? '').toContain('Enter で置く')
})

test('既定表示の案内に ⌘I が載っている', async () => {
  expect(await shortcutsText()).toContain('⌘I — 思いつき')
})

test('I/O「置く」— Enter で置き、閉じずに既定表示へ戻る。現在地もメモの下書きも変わらない', async () => {
  typeNote('接続詞を整える途中 / 次は結論')
  await settle()

  await openCapture()
  typeInto(ideaField() as HTMLInputElement, '請求書の様式を直す')
  press('Enter')
  await settle()

  expect(argsOf('capture_idea')).toEqual({ request: { content: '請求書の様式を直す' } })
  expect(invoked).not.toContain('hide_overlay')
  expect(invoked).not.toContain('switch_current_position')
  expect(invoked).not.toContain('select_step')
  expect(ideaField()).toBeNull()
  expect(document.querySelector('.next-action')?.textContent).toBe('3 段落目を書き直す')
  expect(noteField().value).toBe('接続詞を整える途中 / 次は結論')
  expect(noticeText()).toBe('思いつきを置いた。')
})

test('I/O「休息中に置く」— 置けて、休息は続いている', async () => {
  await enterTheRestingSurface()

  await openCapture()
  expect(ideaField()).not.toBeNull()
  typeInto(ideaField() as HTMLInputElement, '棚を片付ける')
  press('Enter')
  await settle()

  expect(invoked).toContain('capture_idea')
  expect(invoked).not.toContain('end_rest')
  expect(document.body.textContent ?? '').toContain('休息中')
  expect(noticeText()).toBe('思いつきを置いた。')
})

test('I/O「空」— 空白だけでは置かれず、入力欄が残り理由が 1 行出る', async () => {
  await openCapture()
  typeInto(ideaField() as HTMLInputElement, '   ')
  press('Enter')
  await settle()

  expect(ideaField()).not.toBeNull()
  expect(ideaField()?.value).toBe('   ')
  expect(noticeText()).toContain('思いつきを置けなかった')
  expect(noticeText()).toContain('思いつきが空である')
})

test('置くのに失敗したら入力欄を残して理由を示す', async () => {
  captureFails = true
  await openCapture()
  typeInto(ideaField() as HTMLInputElement, '請求書の様式を直す')
  press('Enter')
  await settle()

  expect(ideaField()?.value).toBe('請求書の様式を直す')
  expect(noticeText()).toContain('状態の書き込みに失敗した')
})

test('思いつきの面の Esc は既定表示へ戻るだけで、閉じも置きもしない', async () => {
  typeNote('書きかけ')
  await settle()
  await openCapture()
  typeInto(ideaField() as HTMLInputElement, '捨てる')
  press('Escape')
  await settle()

  expect(ideaField()).toBeNull()
  expect(invoked).not.toContain('capture_idea')
  expect(invoked).not.toContain('hide_overlay')
  expect(noteField().value).toBe('書きかけ')
})

test('I/O「IME」— 変換中の Enter では置かない', async () => {
  await openCapture()
  typeInto(ideaField() as HTMLInputElement, 'せいきゅうしょ')
  press('Enter', { isComposing: true } as KeyboardEventInit)
  await settle()

  expect(invoked).not.toContain('capture_idea')
  expect(ideaField()).not.toBeNull()
})

test('フォーカスを失えば思いつきの下書きは捨てられ、次の呼び出しは初期表示である', async () => {
  await openCapture()
  typeInto(ideaField() as HTMLInputElement, '途中')
  await emit('tauri://blur', null)
  await settle()
  await emit('tauri://focus', null)
  await settle()

  expect(ideaField()).toBeNull()
  expect(invoked).not.toContain('capture_idea')
})

test('受け入れ条件 — 思いつきがあっても、既定表示には本文も数も現れない', async () => {
  ideaList = [IDEA_B, IDEA_A]
  // 一覧では確かに見えている。
  await openDisclosure()
  expect(ideaRows()).toHaveLength(2)

  press('Escape')
  await settle()

  expect(document.querySelector('.disclosure')).toBeNull()
  expect(document.querySelector('.next-action')).not.toBeNull()
  const text = document.body.textContent ?? ''
  expect(text).not.toContain(IDEA_A.content)
  expect(text).not.toContain(IDEA_B.content)
  expect(text).not.toMatch(/思いつき[^·]*\d/)
})

test('I/O「一覧」— 開示面の末尾に見出し「思いつき」と新しい順の行が並ぶ', async () => {
  ideaList = [IDEA_B, IDEA_A]
  await openDisclosure()

  expect(document.querySelector('.ideas-heading')?.textContent?.trim()).toBe('思いつき')
  expect(ideaRows().map((row) => row.textContent?.trim())).toEqual([
    IDEA_B.content,
    IDEA_A.content,
  ])
  // タスクの行の後に並ぶ。
  const all = [...document.querySelectorAll<HTMLElement>('[data-row-key]')].map(
    (row) => row.dataset.rowKey,
  )
  expect(all.slice(-2)).toEqual([`idea:${IDEA_B.ideaId}`, `idea:${IDEA_A.ideaId}`])
})

test('I/O「0 件」— 思いつきが無ければ欄ごと出ない', async () => {
  await openDisclosure()

  expect(document.querySelector('.ideas')).toBeNull()
  expect(document.querySelector('.ideas-heading')).toBeNull()
  expect(ideaRows()).toHaveLength(0)
})

/** 入力位置を、指定の鍵を持つ行まで ↓ で運ぶ。 */
async function moveDownTo(rowKey: string): Promise<void> {
  for (let i = 0; i < 16; i += 1) {
    if ((document.activeElement as HTMLElement)?.dataset.rowKey === rowKey) return
    press('ArrowDown')
    await settle()
  }
  throw new Error(`${rowKey} へ入力位置が届かない`)
}

test('I/O「昇格」— 思いつきの行で Enter → 題名とステップ 1 に文が入り、⌘Enter で昇格する', async () => {
  ideaList = [IDEA_A]
  await openDisclosure()
  await moveDownTo(`idea:${IDEA_A.ideaId}`)

  expect(document.querySelector('.hint')?.textContent ?? '').toContain('Enter でタスクにする')
  press('Enter')
  await settle()

  // 開示面は畳まれ、作成の面が下書きを持って開く。ここでは何も書かれない。
  expect(document.querySelector('.disclosure')).toBeNull()
  expect(titleField().value).toBe(IDEA_A.content)
  expect(stepsField().value).toBe(IDEA_A.content)
  expect(invoked).not.toContain('create_task')

  press('Enter', { metaKey: true })
  await settle()

  expect(argsOf('create_task')).toEqual({
    request: {
      title: IDEA_A.content,
      steps: IDEA_A.content,
      moveCurrentPosition: false,
      ideaId: IDEA_A.ideaId,
    },
  })
  expect(noticeText()).toContain('タスクを作成した')
})

test('I/O「昇格をやめる」— 作成の面を Esc で出れば何も送られず、次の作成は昇格ではない', async () => {
  ideaList = [IDEA_A]
  await openDisclosure()
  await moveDownTo(`idea:${IDEA_A.ideaId}`)
  press('Enter')
  await settle()

  press('Escape')
  await settle()
  expect(invoked).not.toContain('create_task')

  // 普通に作成し直すと、昇格元は持ち越されていない。
  await openCreation()
  expect(titleField().value).toBe('')
  typeInto(titleField(), '別件')
  typeInto(stepsField(), '一')
  press('Enter', { metaKey: true })
  await settle()
  expect((argsOf('create_task')?.request as { ideaId: unknown }).ideaId).toBeNull()
})

test('思いつきの行でもボタンから昇格でき、⌘E は効かない', async () => {
  ideaList = [IDEA_A]
  await openDisclosure()
  await moveDownTo(`idea:${IDEA_A.ideaId}`)

  expect(actionLabels()).toContain('タスクにする')
  expect(actionLabels()).not.toContain('タスク修正')
  press('e', { metaKey: true })
  await settle()
  expect(invoked).not.toContain('get_task_draft')

  actionButton('タスクにする').click()
  await settle()
  expect(titleField().value).toBe(IDEA_A.content)
})

test('タスク 0 件・思いつきありでは「まだタスクが無い。」が思いつきの欄より前に出て、思いつきへ届く', async () => {
  disclosureOverride = { stateError: null, rows: [], ideas: [IDEA_A, IDEA_B] }
  await openDisclosure()

  const empty = document.querySelector('.empty')
  const section = document.querySelector('.ideas')
  expect(empty?.textContent).toBe('まだタスクが無い。')
  expect(section).not.toBeNull()
  expect(
    (empty as Element).compareDocumentPosition(section as Element) &
      Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBeTruthy()

  // 入力位置は先頭の思いつきにあり、↓ で次へ届く。
  expect((document.activeElement as HTMLElement)?.dataset.rowKey).toBe(`idea:${IDEA_A.ideaId}`)
  press('ArrowDown')
  await settle()
  expect((document.activeElement as HTMLElement)?.dataset.rowKey).toBe(`idea:${IDEA_B.ideaId}`)

  press('Enter')
  await settle()
  expect(titleField().value).toBe(IDEA_B.content)
  expect(stepsField().value).toBe(IDEA_B.content)
})

test('置く応答が面を畳んだ後に着いても、既定表示に報せを出さない', async () => {
  holdCapture = true

  await openCapture()
  typeInto(ideaField() as HTMLInputElement, '遅れて着く')
  press('Enter')
  await settle()

  // 飛んでいる間は ⌘I で開き直せない (面を畳んでから押す)。
  await emit('tauri://blur', null)
  await settle()
  press('i', { metaKey: true })
  await settle()
  expect(ideaField()).toBeNull()

  ;(releaseCapture as unknown as () => void)()
  await settle()

  expect(document.body.textContent ?? '').not.toContain('思いつきを置いた。')
})

// ---------------------------------------------------------------------------
// 待機 — 主作業を戻り先として覚えたまま短い別作業をする
// ---------------------------------------------------------------------------

function waitMinutesField(): HTMLInputElement | null {
  return document.querySelector<HTMLInputElement>('input.wait-minutes')
}

function waitLabelField(): HTMLInputElement | null {
  return document.querySelector<HTMLInputElement>('input.wait-label')
}

async function openWait(): Promise<void> {
  press('d', { metaKey: true })
  await settle()
}

/** 待機中の既定表示へ入る。コアは既に待機中である。 */
async function enterTheWaitingSurface(label: string | null = 'AI の出力'): Promise<void> {
  snapshot = { ...SNAPSHOT, waiting: true, waitLabel: label }
  await emit('tauri://focus', null)
  await settle()
  invoked = []
  calls = []
}

test('待機の面は初期表示に無く、⌘D で分数 (既定値) と一行の欄が開く', async () => {
  expect(waitMinutesField()).toBeNull()

  await openWait()

  expect(waitMinutesField()?.value).toBe('5')
  expect(waitLabelField()?.value).toBe('')
  expect(document.activeElement).toBe(waitLabelField())
  expect(document.querySelector('.hint')?.textContent ?? '').toContain('Enter で待機に入る')
  expect(invoked).not.toContain('begin_wait')
})

test('既定表示の案内に ⌘D が載っている', async () => {
  expect(await shortcutsText()).toContain('⌘D — 待機')
})

test('I/O「入る」— 「AI の出力」で Enter → 待機中。次の一手は主作業のまま、閉じない', async () => {
  typeNote('接続詞を整える途中 / 次は結論')
  await settle()
  await openWait()
  typeInto(waitLabelField() as HTMLInputElement, 'AI の出力')
  press('Enter')
  await settle()

  expect(argsOf('begin_wait')).toEqual({ request: { minutes: '5', label: 'AI の出力' } })
  expect(invoked).not.toContain('hide_overlay')
  expect(invoked).not.toContain('switch_current_position')
  expect(waitLabelField()).toBeNull()
  expect(document.querySelector('.next-action')?.textContent).toBe('3 段落目を書き直す')
  expect(document.querySelector('.waiting')?.textContent).toBe('待機中 — AI の出力')
  expect(noteField().value).toBe('接続詞を整える途中 / 次は結論')
  expect(await shortcutsText()).toContain('⌘D — 主作業に戻る')
})

test('「何を待っているか」が空なら null を送り、「待機中」とだけ添える', async () => {
  await openWait()
  typeInto(waitLabelField() as HTMLInputElement, '   ')
  press('Enter')
  await settle()

  expect(argsOf('begin_wait')).toEqual({ request: { minutes: '5', label: null } })
  expect(document.querySelector('.waiting')?.textContent).toBe('待機中')
})

test('I/O「分数が不正」— 「0」「abc」では確定せず、面を残して理由を 1 行示す', async () => {
  for (const broken of ['0', 'abc']) {
    await openWait()
    typeInto(waitMinutesField() as HTMLInputElement, broken)
    press('Enter')
    await settle()

    expect(waitMinutesField()).not.toBeNull()
    expect(waitMinutesField()?.value).toBe(broken)
    expect(noticeText()).toContain('待機に入れなかった')
    expect(noticeText()).toContain('正の整数')
    expect(document.querySelector('.waiting')).toBeNull()

    press('Escape')
    await settle()
  }
})

test('I/O「設定」— 待機の長さが 1 分なら分数の初期値は 1', async () => {
  snapshot = { ...SNAPSHOT, waitLengthMinutes: 1 }
  await emit('tauri://focus', null)
  await settle()

  await openWait()
  expect(waitMinutesField()?.value).toBe('1')
})

test('待機の面の Esc は既定表示へ戻るだけで、閉じも入りもしない', async () => {
  await openWait()
  typeInto(waitLabelField() as HTMLInputElement, 'AI の出力')
  press('Escape')
  await settle()

  expect(waitLabelField()).toBeNull()
  expect(invoked).not.toContain('begin_wait')
  expect(invoked).not.toContain('hide_overlay')
  expect(document.querySelector('.next-action')?.textContent).toBe('3 段落目を書き直す')
})

test('I/O「IME」— 変換中の Enter では待機に入らない', async () => {
  await openWait()
  typeInto(waitLabelField() as HTMLInputElement, 'えーあい')
  press('Enter', { isComposing: true } as KeyboardEventInit)
  await settle()

  expect(invoked).not.toContain('begin_wait')
  expect(waitLabelField()).not.toBeNull()
})

test('フォーカスを失えば待機の下書きは捨てられ、次の呼び出しは初期表示である', async () => {
  await openWait()
  typeInto(waitLabelField() as HTMLInputElement, '途中')
  await emit('tauri://blur', null)
  await settle()
  await emit('tauri://focus', null)
  await settle()

  expect(waitLabelField()).toBeNull()
  expect(invoked).not.toContain('begin_wait')
})

test('I/O「入れない」— 休息中・未着手では ⌘D が何もしない', async () => {
  await enterTheRestingSurface()
  await openWait()
  expect(waitLabelField()).toBeNull()
  expect(invoked).not.toContain('begin_wait')
  expect(invoked).not.toContain('return_from_wait')
  expect(actionLabels()).not.toContain('待機')

  snapshot = { ...SNAPSHOT, stepContent: null, stepOrdinal: null, stepCount: null }
  await emit('tauri://focus', null)
  await settle()
  await openWait()
  expect(waitLabelField()).toBeNull()
  expect(actionLabels()).not.toContain('待機')
})

test('受け入れ条件 — 待機中の既定表示に残り時間・経過・回数が現れない', async () => {
  await enterTheWaitingSurface()

  const text = document.body.textContent ?? ''
  expect(text).toContain('待機中 — AI の出力')
  for (const forbidden of ['残り', '経過', '回目', '%']) expect(text).not.toContain(forbidden)
  // 数字は位置情報 (第 N / 全 M) とホットキーの表記だけである。
  const waitingLine = document.querySelector('.waiting')?.textContent ?? ''
  expect(waitingLine).not.toMatch(/[0-9０-９]/)
})

test('I/O「早めに戻る」— 待機中の ⌘D で書きかけのメモを送り、主作業へ戻る', async () => {
  await enterTheWaitingSurface()
  // 別タスクにいる体で、そのステップのメモを書きかける。
  typeNote('米は 5kg')
  await settle()

  await openWait()

  expect(argsOf('return_from_wait')).toEqual({ request: { note: '米は 5kg' } })
  expect(invoked).not.toContain('begin_wait')
  expect(invoked).not.toContain('hide_overlay')
  expect(document.querySelector('.waiting')).toBeNull()
  // 移ったため下書きは確定済みであり、主作業のメモで初期化し直される。
  expect(noteField().value).toBe('接続詞を整える途中')
  expect(document.activeElement).toBe(noteField())
})

test('早めに戻るときメモを空にしていたら空欄を送る — コアが離れる側のメモを消す', async () => {
  await enterTheWaitingSurface()
  typeNote('')
  await settle()

  await openWait()

  expect(argsOf('return_from_wait')).toEqual({ request: { note: '' } })
})

test('早めに戻るとき、提示されたメモを読み返しただけなら null を送る', async () => {
  await enterTheWaitingSurface()
  await openWait()
  expect(argsOf('return_from_wait')).toEqual({ request: { note: null } })
})

test('戻り先にいたままなら、書きかけのメモは欄に残る', async () => {
  returnOutcome = { returned: true, moved: false }
  await enterTheWaitingSurface()
  typeNote('書きかけ')
  await settle()

  await openWait()

  expect(document.querySelector('.waiting')).toBeNull()
  expect(noteField().value).toBe('書きかけ')
})

test('I/O「早めに戻る」の失敗 — 移らず理由を 1 行示し、閉じない', async () => {
  returnFails = true
  await enterTheWaitingSurface()
  typeNote('米は 5kg')
  await settle()

  await openWait()

  expect(noticeText()).toContain('主作業に戻れなかった')
  expect(noticeText()).toContain('状態の書き込みに失敗した')
  expect(invoked).not.toContain('hide_overlay')
  expect(document.querySelector('.waiting')?.textContent).toBe('待機中 — AI の出力')
  expect(noteField().value).toBe('米は 5kg')
})

test('待機の操作はボタンからも到達できる', async () => {
  actionButton('待機').click()
  await settle()
  expect(waitLabelField()).not.toBeNull()
  expect(actionLabels()).toEqual(['待機に入る', '戻る'])

  actionButton('待機に入る').click()
  await settle()
  expect(invoked).toContain('begin_wait')

  expect(actionLabels()).toContain('主作業に戻る')
  expect(actionLabels()).not.toContain('待機')
  actionButton('主作業に戻る').click()
  await settle()
  expect(invoked).toContain('return_from_wait')
})

test('待機中も一覧から別タスクへ移れる — 待機は続いて見える', async () => {
  await enterTheWaitingSurface()
  await openDisclosure()
  expect(invoked).toContain('get_disclosure_surface')
  expect(invoked).not.toContain('return_from_wait')
})

test('待機の面の案内は分数の欄への行き方を述べ、分数を 10 に変えれば "10" を送る', async () => {
  await openWait()
  expect(document.querySelector('.hint')?.textContent ?? '').toContain('Tab で分数')

  typeInto(waitMinutesField() as HTMLInputElement, '10')
  press('Enter')
  await settle()

  expect(argsOf('begin_wait')).toEqual({ request: { minutes: '10', label: null } })
})

test('待機に入れなかったら、そう述べて面を畳み、待機中の行を描かない', async () => {
  beginWaitOutcome = { waiting: false }
  await openWait()
  press('Enter')
  await settle()

  expect(noticeText()).toContain('待機に入れなかった')
  expect(waitLabelField()).toBeNull()
  expect(document.querySelector('.waiting')).toBeNull()
})

test('待機中でなかったら、そう述べて書きかけのメモを残す', async () => {
  returnOutcome = { returned: false, moved: false }
  await enterTheWaitingSurface()
  typeNote('書きかけ')
  await settle()

  await openWait()

  expect(noticeText()).toContain('待機中ではなかった')
  expect(noteField().value).toBe('書きかけ')
})

