/**
 * 介入パネルの配線を検証する。
 *
 * ここで見るのは「判断」ではなく「配線」である — Rust 側の純粋関数テストは計時の規則と
 * ワイヤ契約を固定するが、二つのボタンが逆に繋がっていても、パネルが自分で消えていても
 * 一つも落ちない。その穴を塞ぐ。
 *
 * **入力先を奪わないことはここでは検証できない。** それは AppKit を起動した実アプリで
 * しか観測できず、本リポジトリは UI 自動化の基盤を持たない (spec の手動手順に残す)。
 */
import { emit } from '@tauri-apps/api/event'
import { clearMocks, mockIPC, mockWindows } from '@tauri-apps/api/mocks'
import { mount, unmount } from 'svelte'
import { afterEach, beforeEach, expect, test } from 'vitest'

import Intervention from './Intervention.svelte'

/** src-tauri/src/commands/mod.rs の `InterventionSnapshot` と 1:1。 */
const SNAPSHOT = {
  hotkey: {
    restAccelerator: 'Control + Option + R',
    graceAccelerator: 'Control + Option + G',
    registered: true,
    error: null,
  },
  stateError: null,
  shown: true,
  kind: 'rest',
}

type EventPluginInternals = { unregisterListener?: (event: string, id: number) => void }

let invoked: string[] = []
let calls: { cmd: string; args: Record<string, unknown> }[] = []
let snapshot: Record<string, unknown> = { ...SNAPSHOT }
let answerOutcome = { answered: true }
/** 真なら取得が失敗する。**IPC を張り替えずに**同じパネルで失敗させるための切り替え。 */
let snapshotFails = false
/** 値があれば取得はこれを返す (宙に浮かせる検査に使う)。 */
let snapshotPending: Promise<unknown> | null = null
let component: Record<string, unknown> | undefined

function stubEventInternals(): void {
  const internals = (
    globalThis as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__: EventPluginInternals }
  ).__TAURI_EVENT_PLUGIN_INTERNALS__
  internals.unregisterListener = () => {}
}

function settle(ms = 0): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function installIPC(): void {
  mockIPC(
    (cmd, args) => {
      invoked.push(cmd)
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> })
      if (cmd === 'get_intervention_snapshot') {
        if (snapshotFails) throw new Error('常駐プロセスが応答しない')
        if (snapshotPending) return snapshotPending
        return snapshot
      }
      if (cmd === 'answer_intervention') return answerOutcome
      return null
    },
    { shouldMockEvents: true },
  )
}

function choices(): HTMLButtonElement[] {
  return [...document.querySelectorAll<HTMLButtonElement>('button.choice')]
}

function choiceLabeled(label: string): HTMLButtonElement {
  const button = choices().find((candidate) =>
    (candidate.querySelector('.label')?.textContent ?? '').includes(label),
  )
  if (!button) throw new Error(`「${label}」の選択肢が無い`)
  return button
}

function argsOf(cmd: string): Record<string, unknown> | undefined {
  return calls.find((call) => call.cmd === cmd)?.args
}

function noticeText(): string {
  return [...document.querySelectorAll('[role="alert"]')]
    .map((node) => node.textContent ?? '')
    .join(' ')
}

async function mountPanel(): Promise<void> {
  document.body.innerHTML = '<div id="app"></div>'
  component = mount(Intervention, {
    target: document.getElementById('app') as HTMLElement,
  })
  await settle()
}

beforeEach(async () => {
  invoked = []
  calls = []
  snapshot = { ...SNAPSHOT }
  answerOutcome = { answered: true }
  snapshotFails = false
  snapshotPending = null
  mockWindows('intervention')
  installIPC()
  stubEventInternals()

  await mountPanel()
  invoked = []
  calls = []
})

afterEach(async () => {
  if (component) unmount(component)
  component = undefined
  await settle()
  clearMocks()
})

test('選択肢はちょうど二つである (FR-15)', () => {
  expect(choices()).toHaveLength(2)
  const labels = choices().map((button) => button.querySelector('.label')?.textContent?.trim())
  expect(labels).toEqual(['休息に入る', '猶予の後に出直す'])
})

test('この面に「閉じる」も「無視する」も無い — 消えるのは応答したときだけである', () => {
  const text = document.body.textContent ?? ''
  expect(text).not.toContain('閉じる')
  expect(text).not.toContain('無視')
  expect(text).not.toContain('あとで')
  // Esc に束縛が無いこと。**押しても何も起きない。**
  window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
  expect(invoked).not.toContain('answer_intervention')
})

test('テキスト入力欄を持たない (AD-6)', () => {
  expect(document.querySelector('input')).toBeNull()
  expect(document.querySelector('textarea')).toBeNull()
})

test('残り時間も経過時間も描かない (spec Never / AD-15)', () => {
  const text = document.body.textContent ?? ''
  for (const forbidden of ['残り', '分後', '経過', '%', '50 分', '15 分']) {
    expect(text).not.toContain(forbidden)
  }
})

test('「休息に入る」を押すと rest が送られる', async () => {
  choiceLabeled('休息に入る').click()
  await settle()

  expect(invoked).toContain('answer_intervention')
  expect(argsOf('answer_intervention')).toEqual({ request: { choice: 'rest' } })
})

test('「猶予の後に出直す」を押すと grace が送られる', async () => {
  choiceLabeled('猶予の後に出直す').click()
  await settle()

  expect(argsOf('answer_intervention')).toEqual({ request: { choice: 'grace' } })
})

test('パネルは自ら隠れない — 閉じるのはコアである (AD-7)', async () => {
  choiceLabeled('休息に入る').click()
  await settle()

  expect(invoked).not.toContain('hide_overlay')
  expect(invoked.filter((cmd) => cmd === 'answer_intervention')).toHaveLength(1)
})

test('応答の途中では二つ目を受けない', async () => {
  mockIPC(
    (cmd, args) => {
      invoked.push(cmd)
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> })
      if (cmd === 'answer_intervention') return new Promise(() => {})
      if (cmd === 'get_intervention_snapshot') return snapshot
      return null
    },
    { shouldMockEvents: true },
  )

  choiceLabeled('休息に入る').click()
  await settle()
  choiceLabeled('猶予の後に出直す').click()
  await settle()

  expect(invoked.filter((cmd) => cmd === 'answer_intervention')).toHaveLength(1)
})

test('応答に失敗したら理由を出し、面は残る', async () => {
  mockIPC(
    (cmd, args) => {
      invoked.push(cmd)
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> })
      if (cmd === 'answer_intervention') throw new Error('書き込めない')
      if (cmd === 'get_intervention_snapshot') return snapshot
      return null
    },
    { shouldMockEvents: true },
  )

  choiceLabeled('休息に入る').click()
  await settle()

  expect(noticeText()).toContain('書き込めない')
  expect(choices()).toHaveLength(2)
})

test('ホットキーを登録できていても、クリックで応答できる形は変わらない (AD-7)', async () => {
  snapshot = {
    ...SNAPSHOT,
    hotkey: { ...SNAPSHOT.hotkey, registered: false, error: '他のアプリが保持している' },
  }
  await emit('intervention_raised', null)
  await settle()

  expect(choices()).toHaveLength(2)
  for (const button of choices()) expect(button.disabled).toBe(false)

  const text = document.body.textContent ?? ''
  expect(text).toContain('クリックで選ぶこと')
  expect(text).toContain('他のアプリが保持している')
  // **効かない打鍵を案内しない。**
  expect(text).not.toContain('Control + Option + R')
})

test('登録できていれば応答の打鍵を案内する', () => {
  const text = document.body.textContent ?? ''
  expect(text).toContain('Control + Option + R')
  expect(text).toContain('Control + Option + G')
})

test('介入が発せられた報せでスナップショットを取り直す (AD-3 鮮度規則)', async () => {
  await emit('intervention_raised', null)
  await settle()

  expect(invoked).toContain('get_intervention_snapshot')
})

test('コアが読めていなければ、面は出したうえで理由を述べる', async () => {
  snapshot = {
    hotkey: null,
    stateError: '保存された状態を読み込めていない。',
    shown: false,
    kind: null,
  }
  await emit('intervention_raised', null)
  await settle()

  expect(noticeText()).toContain('保存された状態を読み込めていない')
  // **面は消えない。** 消えれば、答えるまで消えないという約束のほうが先に破れる。
  expect(choices()).toHaveLength(2)
})

test('スナップショットを取れなくても空白のまま出さない', async () => {
  // **積み直す。** `mockIPC` を張り替えると購読が失われるため、取得が最初から
  // 失敗する状態でパネルを作る。
  if (component) unmount(component)
  component = undefined
  await settle()
  mockIPC(
    (cmd) => {
      invoked.push(cmd)
      throw new Error('常駐プロセスが応答しない')
    },
    { shouldMockEvents: true },
  )
  stubEventInternals()

  await mountPanel()
  // 再試行を使い切るまで待つ (10 回 × 100ms)。
  await settle(1300)

  expect(noticeText()).toContain('常駐プロセスが応答しない')
  // **面は消えない。** 理由も示さずに居座るより、理由を添えて居座るほうが正しい。
  expect(choices()).toHaveLength(2)
}, 10_000)

// --- 復帰の介入 -------------------------------------------------------------

async function raiseReturn(): Promise<void> {
  snapshot = { ...SNAPSHOT, kind: 'return' }
  await emit('intervention_raised', null)
  await settle()
}

test('復帰の介入は「休息を終える」「もう少し休む」の二択を示す', async () => {
  await raiseReturn()

  expect(choices()).toHaveLength(2)
  const labels = choices().map((button) => button.querySelector('.label')?.textContent?.trim())
  expect(labels).toEqual(['休息を終える', 'もう少し休む'])
  expect(document.querySelector('.prompt')?.textContent).toContain('戻る頃合い')
})

test('復帰の介入も同じ二つの打鍵を案内する (AD-7)', async () => {
  await raiseReturn()

  const text = document.body.textContent ?? ''
  expect(text).toContain('Control + Option + R')
  expect(text).toContain('Control + Option + G')
})

test('「休息を終える」は rest、「もう少し休む」は grace を送る', async () => {
  await raiseReturn()

  choiceLabeled('休息を終える').click()
  await settle()
  expect(argsOf('answer_intervention')).toEqual({ request: { choice: 'rest' } })

  calls = []
  choiceLabeled('もう少し休む').click()
  await settle()
  expect(argsOf('answer_intervention')).toEqual({ request: { choice: 'grace' } })
})

test('復帰の介入は数値を一つも描かない (spec Never / AD-15)', async () => {
  await raiseReturn()

  const text = (document.body.textContent ?? '').replaceAll(/Control \+ Option \+ [RG]/g, '')
  expect(text).not.toMatch(/[0-9０-９]/)
  // 「自分」「十分」のような語は禁じない。数 (算用・漢数字) に続く分・秒を禁じる。
  expect(text).not.toMatch(/[0-9０-９〇一二三四五六七八九十百]+\s*(分|秒)/)
  for (const forbidden of ['残り', '経過', '%']) {
    expect(text).not.toContain(forbidden)
  }
})

test('復帰の介入でもパネルは自ら隠れず、オーバーレイも自ら開かない — どちらもコアが行う', async () => {
  await raiseReturn()

  choiceLabeled('休息を終える').click()
  await settle()

  // オーバーレイを開閉するコマンド (`hide_overlay` / `mark_overlay_hidden` など) を
  // パネルは一つも呼ばない。開くのはコアである。
  expect(invoked.filter((cmd) => cmd.includes('overlay'))).toEqual([])
  expect(invoked.filter((cmd) => cmd === 'answer_intervention')).toHaveLength(1)
})

const MEANINGFUL_LABELS = ['休息に入る', '猶予の後に出直す', '休息を終える', 'もう少し休む']

function labels(): string[] {
  return choices().map((button) => button.querySelector('.label')?.textContent?.trim() ?? '')
}

test('種類が分からなければ、どちらの意味も主張しない', async () => {
  snapshot = { ...SNAPSHOT, kind: null }
  await emit('intervention_raised', null)
  await settle()

  expect(choices()).toHaveLength(2)
  for (const label of MEANINGFUL_LABELS) expect(labels()).not.toContain(label)
  expect(labels()).toEqual(['Control + Option + R', 'Control + Option + G'])
})

test('復帰の介入に応答した後、次の介入の取得に失敗しても前の種類の文言を持ち越さない', async () => {
  await raiseReturn()
  choiceLabeled('休息を終える').click()
  await settle()

  // 同じパネルのまま、次の介入の取得だけが失敗する。
  snapshotFails = true
  await emit('intervention_raised', null)
  // 再試行を使い切るまで待つ (10 回 × 100ms)。
  await settle(1300)

  expect(noticeText()).toContain('常駐プロセスが応答しない')
  expect(choices()).toHaveLength(2)
  for (const label of MEANINGFUL_LABELS) expect(labels()).not.toContain(label)
}, 10_000)

test('介入の報せを受けたら、取り直しが済むまで前の種類の文言を出さない', async () => {
  await raiseReturn()
  expect(labels()).toContain('休息を終える')

  let release: (value: unknown) => void = () => {}
  snapshotPending = new Promise((resolve) => (release = resolve))
  await emit('intervention_raised', null)
  await settle()

  for (const label of MEANINGFUL_LABELS) expect(labels()).not.toContain(label)
  release({ ...SNAPSHOT, kind: 'rest' })
  await settle()
  expect(labels()).toEqual(['休息に入る', '猶予の後に出直す'])
})
