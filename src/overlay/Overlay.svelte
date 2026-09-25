<script lang="ts">
  import { onMount, tick } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { listen } from '@tauri-apps/api/event'
  import { getCurrentWindow } from '@tauri-apps/api/window'

  type HotkeyStatus = {
    accelerator: string
    registered: boolean
    error: string | null
  }

  // src-tauri/src/commands/mod.rs の `OverlaySnapshot` と 1:1。
  // `invoke<T>` は実行時検査を行わないため、片方だけ変えても警告は出ない。
  // Rust 側の `tests::the_snapshot_keeps_its_wire_contract` が形を固定している。
  type OverlaySnapshot = {
    hotkey: HotkeyStatus
    stateError: string | null
    stepContent: string | null
    stepOrdinal: number | null
    stepCount: number | null
    interruptionNote: string | null
    resting: boolean
  }

  // src-tauri/src/commands/mod.rs の `BeginRestOutcome` / `EndRestOutcome` と 1:1。
  type BeginRestOutcome = {
    rested: boolean
  }

  type EndRestOutcome = {
    resumed: boolean
  }

  // src-tauri/src/commands/mod.rs の `SwitchRequest` / `SwitchOutcome` と 1:1。
  type SwitchRequest = {
    note: string | null
    declareCompletion: boolean
  }

  type SwitchOutcome = {
    moved: boolean
  }

  // src-tauri/src/commands/mod.rs の `CreateTaskRequest` / `CreateTaskOutcome` と 1:1。
  //
  // **ステップは配列ではなく入力欄の文字列そのものを送る。** 行を切り分けて空行を落とす
  // 規則は一箇所 (Rust 側の `as_task_definition`) にしか無い。ここで切り分ければ、
  // その純粋関数が守っているものが実際の経路から外れる。
  type CreateTaskRequest = {
    title: string
    steps: string
    moveCurrentPosition: boolean
  }

  type CreateTaskOutcome = {
    moved: boolean
  }

  // src-tauri/src/commands/mod.rs の `DisclosureRow` と 1:1。**内部タグ付きである** —
  // `kind` が見出しと行を分ける。見出しに完了の欄が無いのも、確定の意味が二つに分かれる
  // のも、この形そのものが決めている。
  //
  // **見出しの確定はそのタスクを開くだけであり、現在地を動かさない。** ステップの確定
  // だけが現在地を移す。
  type DisclosureRow =
    | {
        kind: 'task'
        taskId: string
        title: string
        open: boolean
        holdsCurrentPosition: boolean
      }
    | { kind: 'step'; stepId: string; content: string; completed: boolean; current: boolean }

  // src-tauri/src/commands/mod.rs の `DisclosureSurface` と 1:1。
  // Rust 側の `tests::the_disclosure_surface_keeps_its_wire_contract` が形を固定している。
  type DisclosureSurface = {
    stateError: string | null
    rows: DisclosureRow[]
  }

  // src-tauri/src/commands/mod.rs の `DisclosureRequest` と 1:1。
  //
  // **ステップを並べるタスクは一つだけである。** `null` は「現在地のタスク」であり、
  // 面を開いた時点の形がこれである。濾すのは射影の側であり、ここではない — 描画側で
  // 濾すと、線に乗った時点で全体像が既に渡っていることになる。
  type DisclosureRequest = {
    openTaskId: string | null
  }

  // src-tauri/src/commands/mod.rs の `SelectStepRequest` / `SelectStepOutcome` と 1:1。
  //
  // **メモも完了も送らない。** この経路は中断メモの機会を与えず、完了にも触れない。
  type SelectStepRequest = {
    stepId: string
  }

  type SelectStepOutcome = {
    moved: boolean
  }

  // src-tauri/src/commands/mod.rs の `CompleteStepRequest` / `CompleteStepOutcome` と 1:1。
  //
  // **移動の欄が無い。** この経路は完了を宣言するだけであり、次にどのステップへ着手
  // するかは利用者が開示面で選ぶ。
  type CompleteStepRequest = {
    note: string | null
  }

  type CompleteStepOutcome = {
    taskFinished: boolean
  }

  // src-tauri/src/commands/mod.rs の `TaskDraftRequest` / `DraftStep` / `TaskDraft` と 1:1。
  //
  // **`taskId` が `null` なら現在地のタスクである。** 開示面の `DisclosureRequest` と
  // 同じ規則であり、決めるのはコアの側である。
  type TaskDraftRequest = {
    taskId: string | null
  }

  type DraftStep = {
    stepId: string
    content: string
  }

  type TaskDraft = {
    stateError: string | null
    taskId: string | null
    title: string
    steps: DraftStep[]
  }

  // src-tauri/src/commands/mod.rs の `EditTaskRequest` と 1:1。
  //
  // **既存のステップだけ ID と本文の対で送る。** 行で送れば対応は並び順という暗黙の
  // 約束になり、1 行ずれた瞬間に別のステップの本文が書き換わる。追記の欄だけは
  // 「1 行 = 1 ステップ」の文字列であり、切り分けるのは Rust 側の `step_contents_of`
  // である (作成と同じ規則)。
  type EditTaskRequest = {
    taskId: string
    title: string
    steps: DraftStep[]
    addedSteps: string
  }

  /** コアから届く、再描画の契機 (AD-3)。状態は運ばれてこない。 */
  const CURRENT_POSITION_CHANGED = 'current_position_changed'

  /** スナップショット取得の再試行回数と間隔。 */
  const SNAPSHOT_RETRIES = 10
  const SNAPSHOT_RETRY_INTERVAL_MS = 100

  let hotkey = $state<HotkeyStatus | null>(null)
  let snapshotError = $state<string | null>(null)
  let stateError = $state<string | null>(null)

  // 以下はすべて**スナップショットの写し**であり、フロントが持つ真実ではない (AD-2)。
  // 表示のたびに取り直され、イベントは取り直しの契機にすぎない (AD-3 鮮度規則)。
  let stepContent = $state<string | null>(null)
  let stepOrdinal = $state<number | null>(null)
  let stepCount = $state<number | null>(null)
  /**
   * 休息中か (CAP-10)。**現在地は値を保ったまま非活性である。**
   *
   * これも**スナップショットの写しである** — フロントは計時をしない (AD-8)。
   * 残り時間も経過時間も運ばれてこない。
   */
  let resting = $state(false)

  /** 休息の宣言が飛んでいる間。二重確定を防ぐ。 */
  let beginningRest = $state(false)
  /** 休息の終了の宣言が飛んでいる間。二重確定を防ぐ。 */
  let endingRest = $state(false)
  /**
   * 休息の終了について述べる 1 行。**休息中の面の外に出す。**
   *
   * 宣言に成功すれば面は「休息中」を畳んで既定表示へ戻る。述べる場所を休息中の面の
   * 中に置くと、「休息中ではなかった」という最も述べる必要のある結末だけが、面ごと
   * 消えて誰にも届かない。
   */
  let restNotice = $state<string | null>(null)
  /** その理由の生の文字列。例外から来たときだけ埋まる。 */
  let restErrorDetail = $state<string | null>(null)

  /**
   * 中断メモの下書き — 揮発ビュー状態 (AD-2)。
   *
   * 確定時にコマンドでコアへ渡す。確定前に失われてよく、閉じたあとの取り直しで
   * 記録済みのメモへ初期化し直される。入力途中の内容は永続化されない (AD-5)。
   */
  let noteDraft = $state('')

  /**
   * 直近のスナップショットが運んできた中断メモ。**下書きの出発点である。**
   *
   * これと下書きが一致している間は「まだ何も書いていない」— 提示されたメモを読み返した
   * だけの切り替えを「メモを書いた」として記録すると、SM-C3 の記入率が膨らむ。記入率は
   * 成功指標の偽陽性 (手放せたのではなく単に書かなくなっただけ) を排除するために存在
   * するので、膨らませるとその役目を失う。
   */
  let notePrefill = $state('')
  let noteInput = $state<HTMLTextAreaElement | null>(null)

  let switchError = $state<string | null>(null)
  let noDestination = $state<{ noteWritten: boolean; completionDeclared: boolean } | null>(null)
  let switching = $state(false)

  /**
   * タスクの下書き — 揮発ビュー状態 (AD-2 の状態表)。
   *
   * 作成の面の開閉と、題名・ステップの入力途中の文字列。確定時にコマンドでコアへ渡す。
   * Esc とフォーカス喪失で破棄されてよく、永続化はされない (AD-5)。
   *
   * **`creating` が初期値で偽であることが FR-2 の実体である。** 作成の面は初期表示に
   * 現れず、明示的な打鍵 (⌘N) を最低 1 回経てのみ到達する (AD-15)。
   */
  let creating = $state(false)
  let titleDraft = $state('')
  let stepsDraft = $state('')
  let titleInput = $state<HTMLInputElement | null>(null)

  let createError = $state<string | null>(null)
  let creatingTask = $state(false)

  /**
   * 修正の面の開閉状態と下書き — 揮発ビュー状態 (AD-2 の状態表)。
   *
   * **`editing` が初期値で偽であることが FR-2 の実体である。** 修正の面は初期表示に
   * 現れず、明示的な打鍵 (⌘E) を最低 1 回経てのみ到達する (AD-15)。Esc・フォーカス
   * 喪失・保存の成功で破棄され、次回の呼び出しは初期表示である。
   *
   * **`editTaskId` はコアが返した値である。** 面へ入るとき `null` で頼めるのは
   * 「現在地のタスク」だけであり、どれが開いたのかを知っているのはコアである。
   * `null` のままなら直す相手が無く、確定の手段そのものを出さない。
   */
  let editing = $state(false)
  let editTaskId = $state<string | null>(null)
  let editTitleDraft = $state('')
  /**
   * 既存のステップの下書き。**ID と本文の対である。**
   *
   * 連番で指さない — 追記や分割で番号が動いた瞬間に別のステップの本文が書き換わる
   * (FR-5)。**消す欄は無い** (v1 に削除の経路が無い)。
   */
  let editStepDrafts = $state<DraftStep[]>([])
  /** 末尾へ追記するステップ。1 行 = 1 ステップ。空でよい。 */
  let editAddedDraft = $state('')
  let editTitleInput = $state<HTMLInputElement | null>(null)
  let editError = $state<string | null>(null)
  let editingTask = $state(false)

  /**
   * 開示面の開閉状態 — 揮発ビュー状態 (AD-2 の状態表)。
   *
   * **`disclosing` が初期値で偽であることが FR-19 の実体である。** 一覧は初期表示に
   * 現れず、明示的な打鍵 (⌘L) を最低 1 回経てのみ到達する (FR-2 / AD-15)。Esc・
   * フォーカス喪失・選択の確定で破棄され、次回の呼び出しは初期表示である。
   *
   * **AD-2 の表に新しい行は要らない。** 開閉状態 (`disclosing` と `openTaskId`) は表の
   * 「開示面の開閉状態」、`rows` はスナップショットの写し (`stepContent` らと同じ)、
   * `selectedRowKey` は表の「フォーカス」に収まる。どれもドメインの真実ではない。
   */
  let disclosing = $state(false)
  let rows = $state<DisclosureRow[]>([])
  /**
   * ステップを並べているタスク — 揮発ビュー状態 (AD-2 の表「開示面の開閉状態」)。
   *
   * **`null` は「現在地のタスク」である。** 面を開いた時点はこれであり、未着手なら
   * どのタスクも開かない。見出しを確定すると、ここがそのタスクへ移る — **一つしか
   * 持てない形が、開いた状態を積み上げられないことそのものである** (spec Boundaries)。
   * 現在地は動かず、切り替え履歴も増えない。
   */
  let openTaskId = $state<string | null>(null)
  /**
   * 入力位置がある行の鍵。**見出しも行である** — 選べるのがステップだけではなくなった。
   *
   * `task:<id>` と `step:<id>` で綴りを分けるのは、ID の集合が交わらないことに寄りかから
   * ないためである。寄りかかれば、同じ文字列の見出しとステップが並んだときに確定の
   * 意味が入れ替わる。
   */
  let selectedRowKey = $state<string | null>(null)
  // **取得の失敗と移動の失敗を一つの変数にまとめない。** まとめると、どちらの文面を
  // 出すかが分岐に紛れ、移動に失敗したときに「一覧を取得できなかった」と述べうる。
  let disclosureError = $state<string | null>(null)
  let selectError = $state<string | null>(null)
  let selecting = $state(false)
  let listElement = $state<HTMLElement | null>(null)

  /**
   * 一覧の取り直しの世代。**遅れて返った応答を捨てるために要る。**
   *
   * `get_disclosure_surface` の応答が `discardDisclosure()` の後に着くと、捨てたはずの
   * 行が書き戻され、次の ⌘L がそれを一瞬見せる。応答は自分が最後の要求であるときだけ
   * 反映する。
   */
  let disclosureRequest = $state(0)

  /**
   * 直近の選択の結末。**「何も書かれていない」を述べる唯一の手がかりである。**
   *
   * 既に現在地である行を選んだとき、面は畳まれ既定表示にはそのステップが出る — 黙って
   * いると、記録されたのかどうかを利用者が確かめる方法が残らない。
   */
  let selected = $state<{ moved: boolean } | null>(null)

  /**
   * 直近の**完了**の宣言の結末。**開示面の上にも出る唯一の報せである。**
   *
   * ⌘Enter は現在地を動かさなくなった。黙って一覧が開けば、完了が記録されたのか、
   * それとも ⌘L を押し間違えただけなのかを確かめる方法が残らない。
   */
  let completionNotice = $state<string | null>(null)

  /** 直近の修正が確定したことを述べる 1 行。面は成功と同時に畳まれる。 */
  let edited = $state(false)

  /** 行を一意に指す鍵。見出しとステップで綴りを分ける。 */
  function rowKey(row: DisclosureRow): string {
    return row.kind === 'task' ? `task:${row.taskId}` : `step:${row.stepId}`
  }

  /** 一覧の全行 — **見出しも含む**。↑↓ はこの並びの上を動く。 */
  const rowKeys = $derived(rows.map(rowKey))

  /** 入力位置がある行そのもの。Enter の意味はこの行の種類で決まる。 */
  const selectedRow = $derived(rows.find((row) => rowKey(row) === selectedRowKey) ?? null)

  /**
   * 直近の作成の結末。**作成が確定したことを述べる唯一の手がかりである。**
   *
   * 面は成功と同時に畳まれるため、述べる場所が無ければ「作成できたのか」を利用者が
   * 確かめる方法が残らない。着手を求めたのに現在地が動かなかった場合は特に重要で、
   * 黙っていると同じ入力がもう一度確定され、v1 では削除も到達もできない重複した
   * タスクが生まれる。
   */
  let created = $state<{ moved: boolean; startRequested: boolean } | null>(null)

  /** 下書きが提示内容から動いているか。動いていなければ「今回は書いていない」。 */
  const noteIsDirty = $derived(noteDraft !== notePrefill)

  /** 移動先が無かったときに、実際に確定したものを述べる 1 行。 */
  const noDestinationNotice = $derived.by(() => {
    if (!noDestination) return ''
    const recorded: string[] = []
    if (noDestination.completionDeclared) recorded.push('完了')
    if (noDestination.noteWritten) recorded.push('メモ')
    const what =
      recorded.length === 0 ? '記録するものは無かった' : `${recorded.join('と')}は記録した`
    return `このタスクに次のステップが無い。${what}が、現在地は動いていない。`
  })

  /** 作成の結末を述べる 1 行。 */
  const creationNotice = $derived.by(() => {
    if (!created) return ''
    if (created.moved) return 'タスクを作成し、現在地をその第 1 ステップへ移した。'
    if (created.startRequested) {
      return 'タスクを作成した。ただし現在地は移せていない — 作成は済んでいるので、同じ入力をもう一度確定しないこと。'
    }
    return 'タスクを作成した。現在地は変えていない。'
  })

  /** 選択の結末を述べる 1 行。移った場合は既定表示そのものが結末であり、何も述べない。 */
  const selectionNotice = $derived(
    selected && !selected.moved ? '現在地は既にそのステップにあった。何も記録していない。' : '',
  )

  /**
   * ボタンの押下が入力位置を奪うのを止める。
   *
   * 中断メモを書きかけたまま押したボタンが欄からフォーカスを外すと、続きが打てなく
   * なる。**click は止めない** — 止めるのは mousedown の既定動作 (フォーカスの移動)
   * だけであり、キーボードから Tab で辿る経路は残る。
   */
  function preventStealingFocus(event: MouseEvent): void {
    event.preventDefault()
  }

  function sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }

  /** 表示できる現在地を持っていない状態へ戻す。 */
  function forgetPosition(): void {
    stepContent = null
    stepOrdinal = null
    stepCount = null
    // **確認できていない休息を描かない。** 残せば、終了の宣言がコアの知らない状態に
    // 対して撃たれる。
    resting = false
    notePrefill = ''
    noteDraft = ''
  }

  /**
   * 一覧の中身だけを捨てる。**面は開いたままである。**
   *
   * コアに確かめられなかった行を残さないための一手であり、`forgetPosition` の一覧側の
   * 対である。残せば、確認できていない行に対して Enter が現在地の移動を撃てる。
   */
  function forgetRows(): void {
    rows = []
    selectedRowKey = null
  }

  // AD-3 (鮮度): 隠れている間に受け取ったイベントに依存せず、表示されるたびに
  // コマンドで完全なスナップショットを取得してから描画する。
  //
  // webview は常駐プロセスの setup が終わる前にも走りうる。その時点では常駐状態が
  // まだ確定しておらずコマンドは拒否を返す。一度の失敗で諦めるとスナップショットが
  // 永久に届かないため、短い間隔で再試行する。
  //
  // `keepDirtyDraft` は、利用者が起こしたのではない取り直し (コアからのイベント) の
  // ためにある。入力途中の下書きを消してよいのはフォーカス離脱と Esc だけである
  // (I/O マトリクス「入力途中の離脱」)。
  async function refresh({ keepDirtyDraft = false } = {}): Promise<void> {
    for (let attempt = 0; attempt < SNAPSHOT_RETRIES; attempt += 1) {
      try {
        const snapshot = await invoke<OverlaySnapshot>('get_overlay_snapshot')
        hotkey = snapshot.hotkey
        snapshotError = null
        stateError = snapshot.stateError
        stepContent = snapshot.stepContent
        stepOrdinal = snapshot.stepOrdinal
        stepCount = snapshot.stepCount
        resting = snapshot.resting === true

        const preserve = keepDirtyDraft && noteIsDirty
        notePrefill = snapshot.interruptionNote ?? ''
        if (!preserve) {
          // 記録済みのメモで初期化し直す。ここが FR-7 の「上書き前の内容の提示」で
          // あり、同時に「入力途中の離脱では下書きが破棄される」の実体でもある。
          noteDraft = notePrefill
          await focusNoteAtEnd()
        }
        // 開示面が出ているなら一覧も取り直す (AD-3 鮮度規則)。**隠れている間の一覧を
        // 持ち越さない。** 入力位置もここで行へ返る — 上の `focusNoteAtEnd` は、この面に
        // 中断メモの欄が無いため何もしていない。
        if (disclosing) await refreshDisclosure()
        return
      } catch (error) {
        if (attempt === SNAPSHOT_RETRIES - 1) {
          console.error('failed to fetch the overlay snapshot', error)
          // 再試行を使い切った。何も描かないと、ホットキーの登録失敗を知らせるために
          // 出したオーバーレイが空白のまま出ることになる。理由を必ず面に出す。
          snapshotError = String(error)
          // **古い現在地を残さない。** 残せば、コアが確認できなかった位置に対して
          // Enter が切り替えを撃てる。
          forgetPosition()
          // **一覧も同じ理由で捨てる。** 常駐が応答しないのに行が残っていれば、
          // Enter がコアの確認できなかった行へ現在地を移しにいく。
          forgetRows()
          return
        }
        await sleep(SNAPSHOT_RETRY_INTERVAL_MS)
      }
    }
  }

  /**
   * 入力欄へフォーカスを移し、カーソルを末尾に置く。
   *
   * **入力位置の制御は面ごとに要る。** 既定表示では中断メモの欄、作成の面では題名の欄が
   * 入力位置を持つ。欄が描かれていなければ何もしない — 排他の分岐により、面に属さない
   * 欄への束縛は `null` である。
   */
  function focusAtEnd(input: HTMLInputElement | HTMLTextAreaElement | null): void {
    if (!input) return
    input.focus()
    const end = input.value.length
    input.setSelectionRange(end, end)
  }

  /**
   * 中断メモの欄へ入力位置を移す (FR-7)。
   *
   * 末尾に置くことが「追記の形を選べる」を成立させる。先頭や全選択にすると、
   * 一打鍵で既存のメモを消してしまう形になる。
   */
  async function focusNoteAtEnd(): Promise<void> {
    await tick()
    focusAtEnd(noteInput)
  }

  /** 題名の欄へ入力位置を移す (I/O マトリクス「面へ入る」)。 */
  async function focusTitleAtEnd(): Promise<void> {
    await tick()
    focusAtEnd(titleInput)
  }

  /**
   * 切り替えの結末として出した報せを畳む。
   *
   * **`refresh()` では畳まない。** 畳む契機は「新しい呼び出し」と「次の確定要求」
   * だけである。
   */
  function dismissNotices(): void {
    switchError = null
    noDestination = null
    created = null
    selected = null
    restNotice = null
    restErrorDetail = null
    completionNotice = null
    edited = false
  }

  /**
   * タスクの下書きを捨て、作成の面を畳む (AD-2 / AD-5)。
   *
   * 入力途中の題名とステップは永続化されない。**畳む契機は Esc・フォーカス喪失・
   * 作成の成功の三つだけである** — コアから届くイベントでは畳まない。
   */
  function discardTaskDraft(): void {
    creating = false
    titleDraft = ''
    stepsDraft = ''
    createError = null
  }

  /**
   * 一覧を捨て、開示面を畳む (AD-2 / FR-19)。**`discardTaskDraft` の双子である。**
   *
   * 畳む契機は Esc・フォーカス喪失・選択の確定の三つだけである。**開閉状態は保持しない**
   * — オーバーレイを閉じた時点で破棄され、次回の呼び出しは初期表示である。
   */
  function discardDisclosure(): void {
    disclosing = false
    forgetRows()
    // 開いていたタスクも持ち越さない。次回の呼び出しで開くのは現在地のタスクである。
    openTaskId = null
    disclosureError = null
    selectError = null
    // 飛んでいる取り直しの応答を無効にする。着いた頃にはこの面は無い。
    disclosureRequest += 1
  }

  /**
   * 一覧を取り直す (CAP-9 / AD-3 鮮度規則)。
   *
   * **表示のたびにコマンドで完全なスナップショットを取得してから描く。** 隠れている間の
   * 一覧を持ち越さない。コアが読めていないことは `stateError` として運ばれる — 面は開き、
   * 理由を示し、移動はできない (I/O マトリクス「コア不在」)。
   */
  async function refreshDisclosure(): Promise<void> {
    const request = (disclosureRequest += 1)
    // **自分が最後の要求でなくなっていたら、何も書かずに帰る。** 面が畳まれた後の
    // 書き戻しは、捨てたはずの行を次の呼び出しへ持ち越す。`stateError` は既定表示と
    // 共有しているため、遅れた応答がそちらの表示まで書き換えうる。
    const isCurrent = (): boolean => request === disclosureRequest && disclosing
    // **開くタスクは要求として送る。** 全部を受け取って描画側で濾す形にすると、線に
    // 乗った時点で全体像が既に渡っており、「見えていないだけ」になる。
    const payload: DisclosureRequest = { openTaskId }
    try {
      const surface = await invoke<DisclosureSurface>('get_disclosure_surface', {
        request: payload,
      })
      if (!isCurrent()) return
      rows = surface.rows
      // **開いたタスクは戻り値から読み直す。** `null` で頼んだときに何が開いたのかを
      // 知っているのはコアだけであり、以降の取り直しはその答えを保つ必要がある —
      // 保たなければ、現在地が動いた報せのたびに開いていたタスクが別のものへ移る。
      openTaskId = openTaskIn(surface.rows)
      // 既定表示と同じ事実であり、同じ一つの変数が持つ。二つに分ければ、同じ「読めて
      // いない」が面によって出たり出なかったりする。
      stateError = surface.stateError
      disclosureError = null
      // 取り直しに成功した時点で、直前の失敗はもうどの行にも結び付かない。
      selectError = null
    } catch (error) {
      if (!isCurrent()) return
      console.error('failed to fetch the disclosure surface', error)
      // **古い一覧を残さない。** 残せば、コアが確認できなかった行に対して Enter が
      // 現在地の移動を撃てる。
      forgetRows()
      disclosureError = String(error)
    }
    keepSelectionOnAnExistingRow()
    await focusSelectedRow()
  }

  /** 一覧で実際に開いている**タスク**。どれも開いていなければ `null`。 */
  function openTaskIn(listed: DisclosureRow[]): string | null {
    for (const row of listed) {
      if (row.kind === 'task' && row.open) return row.taskId
    }
    return null
  }

  /** **現在地**が指す行の鍵。一覧に無ければ (閉じた**タスク**の中なら) `null`。 */
  function keyAtCurrentPosition(): string | null {
    for (const row of rows) {
      if (row.kind === 'step' && row.current) return rowKey(row)
    }
    return null
  }

  /**
   * 選んでいる行を、いま存在する行の上に置き直す。
   *
   * 取り直しで行が消えていれば**現在地**の行へ、それも無ければ先頭へ戻す。指し先を
   * 失ったまま残すと、Enter が何も起こさない状態になる。
   */
  function keepSelectionOnAnExistingRow(): void {
    if (selectedRowKey !== null && rowKeys.includes(selectedRowKey)) return
    // **現在地**の行が見えていればそこへ。見えていなければ (どのタスクも開いていない
    // ときを含む) 先頭の見出しへ置く。
    selectedRowKey = keyAtCurrentPosition() ?? rowKeys[0] ?? null
  }

  /**
   * 入力位置を選んでいる行へ移す。**この面は入力欄ではなく行に焦点を置く。**
   *
   * 窓は固定寸法である。溢れた行を選んだときにスクロールで到達できるよう、選んだ行を
   * 見える位置まで送る。
   */
  async function focusSelectedRow(): Promise<void> {
    await tick()
    if (!listElement || selectedRowKey === null) return
    // **鍵をセレクタへ埋め込まない。** 引用符や逆斜線を含む値が来れば
    // `querySelector` は投げ、ここは try の外であるため未処理の rejection になる。
    const row = [...listElement.querySelectorAll<HTMLElement>('[data-row-key]')].find(
      (candidate) => candidate.dataset.rowKey === selectedRowKey,
    )
    if (!row) return
    row.focus()
    row.scrollIntoView?.({ block: 'nearest' })
  }

  /**
   * 開示面へ入る (CAP-9 / FR-19)。
   *
   * **初期表示には現れない。** ここへ到達する経路は明示的な打鍵だけである (FR-2 / AD-15)。
   * 入力位置は**現在地**の行に置かれる (I/O マトリクス「面へ入る」)。
   */
  async function openDisclosure({ keepNotices = false } = {}): Promise<void> {
    if (disclosing) return
    // **`keepNotices` は完了の宣言のためにある。** 宣言の直後にこの面を開くのは
    // 「次に着手するステップを選ぶ」ためであり、そこで報せを畳めば、完了が記録された
    // ことを述べる場所が一つも残らない。
    if (!keepNotices) dismissNotices()
    disclosing = true
    // **面を開いた時点で開いているのは現在地のタスクである** (spec Boundaries)。
    // どれを開くかを決めるのはコアであり、未着手ならどれも開かない。
    openTaskId = null
    await refreshDisclosure()
  }

  /**
   * 開示面から出る。**オーバーレイは閉じない** (I/O マトリクス「Esc」)。
   *
   * 既定表示へ戻り、入力位置を中断メモの欄へ返す。
   *
   * **戻り際に必ず取り直す。** 一覧の取得は `stateError` を書き換えるため、取り直さずに
   * 戻ると、直前のスナップショットの失敗で消えた現在地が「未着手」として描かれたまま、
   * 理由の行だけが消えた既定表示になる。
   *
   * **入力途中の中断メモは残す。** ここでの Esc は面から戻る操作であって、オーバーレイ
   * を閉じる Esc ではない (`leaveCreation` と同じ扱い)。
   */
  async function leaveDisclosure(): Promise<void> {
    discardDisclosure()
    await refresh({ keepDirtyDraft: true })
    await focusNoteAtEnd()
  }

  /**
   * 隣の行へ入力位置を移す。**端では留まる** — 回り込ませると、一覧のどこにいるのかが
   * 分からなくなる。**見出しも止まる行である** — 読み飛ばすと、閉じている**タスク**を
   * 開く手段が無くなる。
   */
  async function moveSelection(offset: 1 | -1): Promise<void> {
    if (rowKeys.length === 0) return
    const at = selectedRowKey === null ? -1 : rowKeys.indexOf(selectedRowKey)
    const next = Math.min(Math.max(at + offset, 0), rowKeys.length - 1)
    await selectRow(rowKeys[next])
  }

  /**
   * 行を選ぶ。**確定はしない** — マウスの click もここへ来る。
   *
   * 押した瞬間に現在地を移さないのは、この面が「選んでから確定する」一つの形しか
   * 持たないためである。click を確定にすると、Space や修飾キーを伴う click まで
   * ボタンの既定動作として確定に化け、Enter の分岐が意図して拒んでいる組み合わせが
   * マウス経由で通ってしまう。
   */
  async function selectRow(key: string): Promise<void> {
    selectedRowKey = key
    // 直前の失敗は選び直した行には結び付かない。
    selectError = null
    await focusSelectedRow()
  }

  /**
   * 入力位置がある行を確定する。**行の種類が Enter の意味を決める。**
   *
   * 見出しならその**タスク**を開くだけであり、**現在地**は動かず**切り替え履歴**も
   * 増えない。**ステップ**なら**現在地**がそこへ移る (spec Boundaries)。
   */
  async function confirmSelection(): Promise<void> {
    const row = selectedRow
    // 確定の途中では二つ目を受けない。取り直しの最中に開く先が変わると、着いた応答が
    // どちらの要求のものか読めなくなる。
    if (!row || selecting) return
    if (row.kind === 'task') {
      await openTask(row.taskId)
      return
    }
    await moveTo(row.stepId)
  }

  /**
   * 見出しを確定し、その**タスク**の**ステップ**を並べる (spec Boundaries)。
   *
   * **直前に開いていたタスクは閉じる。** 開いた状態を積み上げられないことが、一覧が
   * 全体像へ戻らないための条件である — 開くタスクを一つしか持てない形がそれを負う。
   *
   * **何も書かない。** 現在地は動かず、切り替え履歴も増えない。取り直すのは一覧だけで
   * あり、既定表示のスナップショットには触れない。
   *
   * 入力位置は確定した見出しに留まる。開いた**ステップ**へ飛ばすと、確定の手応えを
   * 確かめるもう一度の Enter が、見ていない**ステップ**への**現在地**の移動になる。
   *
   * **既に開いている見出しを確定しても閉じない。** 畳む打鍵を与えれば、閉じた一覧から
   * 数打鍵で全体像へ戻る形が生まれる — 開けるのが一つだけであることの意味が薄れる。
   * 取り直しだけが走る (AD-3 鮮度規則)。
   */
  async function openTask(taskId: string): Promise<void> {
    openTaskId = taskId
    selectError = null
    await refreshDisclosure()
  }

  /**
   * 選んだステップへ現在地を移す (CAP-9 / FR-19)。
   *
   * 用語集の定義によりこれは**切り替え**であり、現在地の移動と切り替え履歴の追記は
   * コア側の**単一のトランザクション**で確定する (AD-5)。**ただしこの経路は中断メモの
   * 機会を与えない** — 挟めば CAP-7 の儀式を一覧の中に作り直すことになる。
   *
   * 既に現在地である行を選んだときはコアが何も書かない。そのことは結末の `moved` として
   * 返り、既定表示に 1 行として出る — 黙っていると、記録されたのかどうかを確かめる方法が
   * 残らない。
   *
   * # 入力途中の中断メモは捨てる
   *
   * **ここは下書きを捨てる三つ目の経路である** (他はオーバーレイを閉じる Esc とフォーカス
   * 喪失 / AD-2)。下書きは離れたステップに向けて書かれたものであり、持ち越せば別の
   * ステップの欄に前の文脈が載ったまま、次の切り替えでそれが確定しうる。**この経路は
   * メモの機会を与えない**以上、下書きを生かしておく先が無い。面から戻るだけの Esc
   * ([`leaveDisclosure`]) は現在地を動かさないため、そちらでは残す。
   */
  async function moveTo(stepId: string): Promise<void> {
    if (selecting) return
    selecting = true
    selectError = null
    const request: SelectStepRequest = { stepId }
    try {
      const outcome = await invoke<SelectStepOutcome>('select_step', { request })
      // 面を出て、取り直した既定表示がそのステップを示す。**オーバーレイは閉じない** —
      // 移った先の次の一手と中断メモを、そのまま見るための面である (FR-8)。
      discardDisclosure()
      await refresh()
      await focusNoteAtEnd()
      selected = { moved: outcome.moved }
    } catch (error) {
      console.error('failed to select the step', error)
      // **消えた行を選べるまま残さない。** 残せば、同じ Enter が同じ失敗を繰り返す。
      // 取り直しは成功時に `selectError` を消すため、理由はその後に置く。
      await refreshDisclosure()
      // **面は閉じず、理由をその場に出す。** 閉じれば、どの行で何が起きたのか分からない。
      selectError = String(error)
    } finally {
      selecting = false
    }
  }

  /**
   * 修正の面へ入る (CAP-5 / FR-4 / FR-5)。
   *
   * **初期表示には現れない。** ここへ到達する経路は明示的な打鍵 (⌘E) と、同じ意味の
   * ボタンだけである (FR-2 / AD-15)。
   *
   * `taskId` が `null` なら**現在地**の**タスク**を直す — 既定表示から入ったときの形が
   * これである。開示面から入ったときは、選んでいる行が属する**タスク**を名指しする。
   *
   * **開示面は畳む。** 二つの面を同時に出さない (作成の面と同じ排他である)。
   */
  async function openEdit(taskId: string | null): Promise<void> {
    if (editing) return
    dismissNotices()
    // 一覧から来た場合はそれを畳む。飛んでいる取り直しの応答もここで無効になる。
    discardDisclosure()
    editing = true
    editError = null
    await refreshDraft(taskId)
  }

  /**
   * 下書きの出発点をコアから取り直す (CAP-5 / AD-3 鮮度規則)。
   *
   * **いまの題名と本文はコアだけが知っている。** 一覧の行から組み立てると、閉じた
   * **タスク**には**ステップ**の行が無く、直す相手の中身が揃わない。
   */
  async function refreshDraft(taskId: string | null): Promise<void> {
    const payload: TaskDraftRequest = { taskId }
    try {
      const draft = await invoke<TaskDraft>('get_task_draft', { request: payload })
      // 既定表示と同じ事実であり、同じ一つの変数が持つ (`refreshDisclosure` と同じ)。
      stateError = draft.stateError
      // **開いた相手は戻り値から読み直す。** `null` で頼んだときに何が開いたのかを
      // 知っているのはコアだけである。
      editTaskId = draft.taskId
      editTitleDraft = draft.title
      // **写しを作る。** そのまま束縛すると、入力のたびにスナップショットの中身を
      // 書き換えていることになる。
      editStepDrafts = draft.steps.map((step) => ({ ...step }))
      editAddedDraft = ''
      editError = null
    } catch (error) {
      console.error('failed to fetch the task draft', error)
      // **確認できていない相手を残さない。** 残せば、コアが確かめられなかった ID に
      // 対して保存が撃たれる。
      editTaskId = null
      editTitleDraft = ''
      editStepDrafts = []
      editError = String(error)
    }
    await focusEditTitleAtEnd()
  }

  /** 修正の面の題名の欄へ入力位置を移す (I/O マトリクス「面へ入る」)。 */
  async function focusEditTitleAtEnd(): Promise<void> {
    await tick()
    focusAtEnd(editTitleInput)
  }

  /**
   * 下書きを捨て、修正の面を畳む (AD-2 / AD-5)。**`discardTaskDraft` の双子である。**
   *
   * 畳む契機は Esc・フォーカス喪失・保存の成功の三つだけである。
   */
  function discardTaskEdit(): void {
    editing = false
    editTaskId = null
    editTitleDraft = ''
    editStepDrafts = []
    editAddedDraft = ''
    editError = null
  }

  /**
   * 修正の面から出る。**オーバーレイは閉じない** (I/O マトリクス「Esc」)。
   *
   * **戻り際に必ず取り直す。** 下書きの取得は `stateError` を書き換えるため、取り直さずに
   * 戻ると、直前の失敗で消えた現在地が「未着手」として描かれたまま残る
   * (`leaveDisclosure` と同じ理由)。**入力途中の中断メモは残す。**
   */
  async function leaveEdit(): Promise<void> {
    discardTaskEdit()
    await refresh({ keepDirtyDraft: true })
    await focusNoteAtEnd()
  }

  /**
   * タスクを直す (CAP-5 / FR-4 / FR-5)。
   *
   * 題名・既存のステップの本文・末尾への追記は、コア側の**単一のトランザクション**で
   * 確定する (AD-5)。**現在地・完了・中断メモのいずれにも触れない。**
   *
   * 題名の欠落と空のステップは Rust 側の純粋関数が判定する。ここで先回りして弾くと、
   * 規則が二箇所に分かれ、片方だけが直る形になる (`confirmCreation` と同じ流儀)。
   */
  async function confirmEdit(): Promise<void> {
    // **相手が無ければ確定しない。** `null` を送れば、読めない ID として拒まれるだけで
    // ある。
    if (editingTask || editTaskId === null) return
    editingTask = true
    editError = null
    const request: EditTaskRequest = {
      taskId: editTaskId,
      title: editTitleDraft,
      steps: editStepDrafts.map((step) => ({ stepId: step.stepId, content: step.content })),
      addedSteps: editAddedDraft,
    }
    try {
      await invoke('edit_task', { request })
      discardTaskEdit()
      // `task_created` の購読と同じ規則で取り直す (`confirmCreation` と同じ理由)。
      await refresh({ keepDirtyDraft: true })
      await focusNoteAtEnd()
      edited = true
    } catch (error) {
      // **面は閉じず、入力も保持したまま理由を示す** (I/O マトリクス)。閉じれば、
      // 打ち直した題名と本文が理由もろとも消える。
      console.error('failed to edit the task', error)
      editError = String(error)
    } finally {
      editingTask = false
    }
  }

  /**
   * 開示面で直す相手にあたる**タスク**。
   *
   * 見出しを選んでいればその**タスク**、**ステップ**の行を選んでいれば開いている
   * **タスク**である — **ステップ**の行が現れるのは開いた**タスク**の分だけであり、
   * 属する先は一つに定まる。
   */
  const taskToEdit = $derived(
    selectedRow?.kind === 'task' ? selectedRow.taskId : (openTaskId ?? null),
  )

  /**
   * 作成の面へ入る (CAP-4 / FR-2)。
   *
   * **初期表示には現れない。** ここへ到達する経路は明示的な打鍵だけである (AD-15)。
   * 既存のタスクを並べて選ばせる面は作らない — それは CAP-9 の開示面である (FR-19)。
   */
  async function openCreation(): Promise<void> {
    if (creating) return
    dismissNotices()
    creating = true
    createError = null
    await focusTitleAtEnd()
  }

  /**
   * 作成の面から出る。**オーバーレイは閉じない** (I/O マトリクス「Esc」)。
   *
   * 既定表示へ戻り、入力位置を中断メモの欄へ返す。
   */
  async function leaveCreation(): Promise<void> {
    discardTaskDraft()
    await focusNoteAtEnd()
  }

  /**
   * タスクを作る (CAP-4 / FR-4)。
   *
   * `moveCurrentPosition` が真なら、作成に加えて現在地をその第 1 ステップへ置く。
   * **書き留めることと着手することは別の行為である** — 打鍵で選ばせることで、作業中に
   * 思いついたタスクを書き留めるだけの経路と、いま始める経路の両方を持てる。
   *
   * 題名の欠落とステップの不足は Rust 側の純粋関数が判定する。ここで先回りして弾くと、
   * 規則が二箇所に分かれ、片方だけが直る形になる。
   */
  async function confirmCreation(moveCurrentPosition: boolean): Promise<void> {
    if (creatingTask) return
    creatingTask = true
    createError = null
    const request: CreateTaskRequest = {
      title: titleDraft,
      steps: stepsDraft,
      moveCurrentPosition,
    }
    try {
      const outcome = await invoke<CreateTaskOutcome>('create_task', { request })
      // 面を出て、スナップショットを取り直した既定表示へ戻る (AD-3 鮮度規則)。
      // 取りこぼしても次回表示で正しくなる。
      discardTaskDraft()
      // **`keepDirtyDraft` は `current_position_changed` の購読と同じ値でなければ
      // ならない。** 着手を伴う作成はそのイベントを発行し、購読側の取り直しがこの
      // 取り直しと競走する。二つが同じ規則で走る限り、どちらが後に解決しても結果は
      // 同じになる。違えれば、勝った側によって中断メモの下書きが残ったり消えたりする。
      // 下書きを捨ててよいのはフォーカス離脱と Esc だけである。
      await refresh({ keepDirtyDraft: true })
      // 入力位置を中断メモの欄へ返す。`refresh` は下書きを保った場合に入力位置へ
      // 触れないため、ここで明示的に戻す。
      await focusNoteAtEnd()
      // **作成は確定している。** `moved` が偽でも作り直させない。
      created = { moved: outcome.moved, startRequested: moveCurrentPosition }
    } catch (error) {
      // **面は閉じず、入力も保持したまま理由を示す** (I/O マトリクス)。閉じれば、
      // 打ち込んだ題名とステップが理由もろとも消える。
      console.error('failed to create the task', error)
      createError = String(error)
    } finally {
      creatingTask = false
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

  /**
   * 休息に入ることを宣言する (CAP-10 / FR-15)。
   *
   * **介入を待たない。** 介入は「自力では休息を取れていない」を補う働きかけであって、
   * 休息へ入る唯一の門ではない。閾値より前に離席する日は当然にあり、宣言する手段が
   * 無ければ離席の間も連続作業時間が伸び、戻った直後に介入が出る — 反射的に無視される
   * 介入は、介入全体の信頼性を損なう。
   *
   * 現在地は値を保ったまま非活性になり、計数が止まる。**オーバーレイは閉じない** —
   * 取り直した面が「休息中」に入れ替わることが、宣言が効いた唯一の手応えである。
   */
  async function declareRest(): Promise<void> {
    // 現在地が無ければ止める計数も無い。既に休息中なら宣言する相手も無い。
    if (stepContent === null || beginningRest || resting) return
    beginningRest = true
    dismissNotices()
    try {
      const outcome = await invoke<BeginRestOutcome>('begin_rest')
      // 取り直しが次の一手を畳み、「休息中」に置き換える (AD-3 鮮度規則)。
      await refresh()
      if (!outcome.rested) {
        // **何も書かれていない。** 面も入れ替わらないため、述べなければ手応えが残らない。
        restNotice = '休息に入れなかった。現在地が無いか、既に休息中である。'
      }
    } catch (error) {
      console.error('failed to declare the rest', error)
      restNotice = '休息に入れなかった。状態は変わっていない。'
      restErrorDetail = String(error)
    } finally {
      beginningRest = false
    }
  }

  /**
   * 休息の終了を宣言する (CAP-10 / FR-15)。
   *
   * **休息の終了はユーザーの明示的な宣言による。** 時間でも、オーバーレイを開いた
   * ことでも終わらない — 開いただけで終わるなら、休息中に予定を確かめることが
   * できなくなる。
   *
   * 宣言の後は現在地が活性へ戻り、取り直した既定表示が CAP-8 と同じ形式で位置と
   * 中断メモを出す。**オーバーレイは閉じない** — 戻った先を読むための面である。
   */
  async function declareEndOfRest(): Promise<void> {
    if (endingRest) return
    endingRest = true
    dismissNotices()
    try {
      const outcome = await invoke<EndRestOutcome>('end_rest')
      // 取り直しが「休息中」を畳み、次の一手と位置情報に置き換える (AD-3 鮮度規則)。
      await refresh()
      await focusNoteAtEnd()
      if (!outcome.resumed) {
        // 既に活性だった。**何も書かれていない。**
        restNotice = '休息中ではなかった。何も変えていない。'
      }
    } catch (error) {
      console.error('failed to declare the end of the rest', error)
      restNotice = '休息を終えられなかった。状態は変わっていない。'
      restErrorDetail = String(error)
    } finally {
      endingRest = false
    }
  }

  /**
   * 完了を宣言し、次に着手するステップを選ぶ (CAP-7 / FR-4 / FR-19)。
   *
   * 離脱側のメモの確定と**完了**の宣言は、コア側の**単一のトランザクション**で確定する
   * (AD-5)。**現在地は動かない。**
   *
   * # なぜ次のステップへ自動で進めないのか
   *
   * **完了**の次にどこへ行くかを決めるのは利用者である。同一タスク内の次のステップへ
   * 自動で進めば、そのタスクを続ける以外の選択が「進んでしまった後の訂正」になり、
   * 訂正のたびに実際には作業していないステップからの離脱が切り替え履歴に残る。
   *
   * # なぜオーバーレイを閉じないのか
   *
   * 選ぶ面をここで開くためである。閉じてしまえば、選ぶために呼び出し直すことになる。
   * 選択の確定 ([`moveTo`]) もオーバーレイを閉じない — 移った先の次の一手と中断メモを
   * そのまま見るための面である (FR-8)。
   */
  async function completeAndChooseNext(): Promise<void> {
    // 現在地が無ければ宣言する相手も無い。二重確定も防ぐ。**休息中は宣言しない** —
    // その面の Enter は休息の終了の宣言だけを意味する。
    if (stepContent === null || switching || resting) return
    switching = true
    dismissNotices()
    // **提示されたメモをそのまま送り返さない** (`confirmSwitch` と同じ規則)。
    const request: CompleteStepRequest = { note: noteIsDirty ? noteDraft : null }
    try {
      const outcome = await invoke<CompleteStepOutcome>('complete_current_step', { request })
      // 完了の印とメモを取り直してから一覧を開く (AD-3 鮮度規則)。
      await refresh()
      completionNotice = outcome.taskFinished
        ? '完了を宣言した。このタスクはこれで終わりであり、現在地が離れれば一覧から消える。'
        : '完了を宣言した。次に着手するステップを選ぶこと。'
      await openDisclosure({ keepNotices: true })
    } catch (error) {
      console.error('failed to declare the completion', error)
      switchError = String(error)
    } finally {
      switching = false
    }
  }

  /**
   * 切り替えを確定させる (CAP-7)。
   *
   * 離脱側のメモ確定・完了宣言 (任意)・現在地の移動・切り替え履歴の追記は、コア側の
   * **単一のトランザクション**で確定する (AD-5)。ここが行うのは要求と、その結末に
   * 応じた離脱だけである。
   */
  async function confirmSwitch(): Promise<void> {
    // 現在地が無ければ離れるべき場所も無い。二重確定も防ぐ。
    //
    // **休息中は切り替えない。** 切り替えれば現在地が活性へ戻り、休息が選ばれても
    // いないのに黙って終わる (AD-8 のリセット契機を宣言なしに踏む)。この面の Enter は
    // 休息の終了の宣言だけを意味する。
    if (stepContent === null || switching || resting) return
    switching = true
    dismissNotices()
    // **提示されたメモをそのまま送り返さない。** 送り返せば、読み返しただけの
    // 切り替えが「メモを書いた」として履歴に残り、SM-C3 の記入率が膨らむ。
    // **この経路は完了を宣言しない。** 宣言を伴う打鍵は [`completeAndChooseNext`] へ
    // 分かれた。欄そのものはコア側の契約に残っている — **完了**を伴う**切り替え**は
    // ドメインの操作として成立し続けるためであり、ここが送らないだけである。
    const request: SwitchRequest = {
      note: noteIsDirty ? noteDraft : null,
      declareCompletion: false,
    }
    try {
      const outcome = await invoke<SwitchOutcome>('switch_current_position', { request })
      if (outcome.moved) {
        await close()
        return
      }
      // 最終ステップだった。取り直してから、実際に確定したものを述べる。
      await refresh()
      noDestination = {
        // 空白だけの本文はコア側で省略として扱われる。述べる内容をそれに合わせる。
        noteWritten: (request.note ?? '').trim() !== '',
        // **この経路は完了を宣言しない。** 宣言を伴う打鍵は別の経路へ分かれた。
        completionDeclared: false,
      }
    } catch (error) {
      console.error('failed to commit the switch', error)
      switchError = String(error)
    } finally {
      switching = false
    }
  }

  /**
   * ⌘ と綴りだけで決まる打鍵か。他の修飾キーを伴えば別の打鍵である。
   *
   * **面へ入る三つ (⌘N / ⌘L / ⌘E) と、休息の宣言 (⌘R) が同じ形を共有する。** 綴りごとに
   * 判定を書き分けると、CapsLock の扱いのような細部が片方にだけ入る。
   *
   * **`key` を畳んで比べる。** CapsLock が入っていると `key` は `'L'` で届き、小文字と
   * の比較では一致しない — 案内どおりに押しても何も起きない面ができる。
   */
  function isSurfaceKey(event: KeyboardEvent, letter: 'n' | 'l' | 'e' | 'r'): boolean {
    return (
      event.key.toLowerCase() === letter &&
      event.metaKey &&
      !event.shiftKey &&
      !event.altKey &&
      !event.ctrlKey
    )
  }

  function onKeydown(event: KeyboardEvent): void {
    // **IME の変換中は一切割り込まない** (AD-6)。日本語入力中の Enter は変換の確定、
    // Esc は変換の取り消しであって、切り替えの宣言でも離脱でもない。Esc をここより
    // 後ろで見ると、変換を取り消したつもりでオーバーレイが閉じ、下書きが消える。
    // `keyCode === 229` は isComposing を立てない環境向けの保険である。
    if (event.isComposing || event.keyCode === 229) return

    // 作成の面は自前の打鍵を持つ。既定表示の Enter (切り替え) をここへ持ち込まない —
    // 入力中の Enter が切り替えを撃てば、下書きごと面が消える。
    if (creating) {
      onCreationKeydown(event)
      return
    }

    // 開示面も自前の打鍵を持つ。既定表示の Enter (切り替え) をここへ持ち込まない —
    // 一覧の Enter は選んだ行の確定であって、同一タスク内の次のステップへの切り替え
    // ではない。**この分岐が作成の面との排他でもある** — どちらの面も、出ている間は
    // 相手へ入る打鍵に触れさせない。
    if (disclosing) {
      onDisclosureKeydown(event)
      return
    }

    // 修正の面も自前の打鍵を持つ。既定表示の Enter (切り替え) をここへ持ち込まない —
    // 入力中の Enter が切り替えを撃てば、打ち直した題名と本文ごと面が消える。
    if (editing) {
      onEditKeydown(event)
      return
    }

    if (event.key === 'Escape') {
      // 確定の途中では閉じない。閉じてしまうと、失敗したときの理由を読む機会が
      // 画面ごと消える。
      if (switching || endingRest) return
      event.preventDefault()
      // 下書きは確定されない。永続化もされない (AD-5)。
      void close()
      return
    }

    // 面へ入る二つの打鍵 (FR-2 / FR-19 / AD-15)。**初期表示から作成の面にも一覧にも
    // 明示操作を最低 1 回経ずには到達しない。**
    //
    // **打鍵を飲むのが先である。** 確定の途中で先に return すると、押した ⌘N / ⌘L が
    // webview の既定動作へ抜ける。確定の途中で面へ入らないのは、入れば失敗したときの
    // 理由を読む機会が面ごと消えるためである。
    if (isSurfaceKey(event, 'n')) {
      event.preventDefault()
      if (switching) return
      void openCreation()
      return
    }

    if (isSurfaceKey(event, 'l')) {
      event.preventDefault()
      if (switching) return
      void openDisclosure()
      return
    }

    // 三つ目の面 (CAP-5 / FR-4 / FR-5)。**直す相手は現在地のタスクである** — どれを
    // 直すかを選ばせる面は作らない。それは CAP-9 の開示面であり、そちらからも同じ
    // 打鍵で入れる。
    if (isSurfaceKey(event, 'e')) {
      event.preventDefault()
      if (switching) return
      // **現在地が無ければ直す相手も無い。** 面だけ開いて「相手が無い」と述べるより、
      // 打鍵が何も起こさないほうが案内と食い違わない。
      if (stepContent === null) return
      void openEdit(null)
      return
    }

    // 休息の宣言 (CAP-10 / FR-15)。**面へは入らない** — 取り直した既定表示が
    // 「休息中」に入れ替わる。
    if (isSurfaceKey(event, 'r')) {
      event.preventDefault()
      if (switching) return
      void declareRest()
      return
    }

    if (event.key !== 'Enter') return
    // **押しっぱなしの自動反復を確定として扱わない。** 作成の面は成功と同時に畳まれる
    // ため、押し続けられた ⌘Enter の続きがそのまま既定表示へ落ち、生まれたばかりの
    // ステップに対して完了の宣言と切り替えを撃つ。
    if (event.repeat) return
    // Shift+Enter は改行。Option / Control を伴う Enter は受け付けない — 受け付けると
    // 案内 (⌘Enter) と実際に効く打鍵が食い違う。
    if (event.shiftKey || event.altKey || event.ctrlKey) return

    event.preventDefault()
    // **休息中の Enter は休息の終了の宣言である** (CAP-10)。切り替えではない。
    if (resting) {
      void declareEndOfRest()
      return
    }
    // 完了の宣言は同じ一連の操作から 1 打鍵で到達する (FR-4)。修飾キーの有無だけが
    // 違う。
    //
    // **二つは移動の扱いが違う。** 素の Enter は同一タスク内の次のステップへ移って
    // 離脱する。⌘Enter は完了を宣言してそこに留まり、次に着手するステップを一覧から
    // 選ばせる — 完了の次にどこへ行くかを決めるのは利用者である。
    if (event.metaKey) {
      void completeAndChooseNext()
      return
    }
    void confirmSwitch()
  }

  /**
   * 作成の面の打鍵。**IME ガードは呼び出し元が既に通している** (AD-6)。
   *
   * # なぜ確定が素の Enter ではないのか
   *
   * ステップの入力欄は 1 行 = 1 ステップであり、複数行を打つことが前提である。素の
   * Enter を確定にすると、2 行目を打とうとした打鍵がそのままタスクを生む。v1 には
   * 削除の経路が無く (CAP-5 も本スライスに無い)、生まれた 1 ステップのタスクは
   * 取り消せない。改行を素の Enter に残し、確定には ⌘ を要求する。
   */
  function onCreationKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      // 確定の途中では出ない。失敗したときの理由を読む機会が消える。
      if (creatingTask) return
      event.preventDefault()
      // 下書きは確定されない。永続化もされない (AD-5)。
      void leaveCreation()
      return
    }

    if (event.key !== 'Enter') return
    // 押しっぱなしの自動反復を確定として扱わない (既定表示と同じ理由)。
    if (event.repeat) return
    // 素の Enter と Shift+Enter は改行である。Option / Control は受け付けない —
    // 受け付けると案内と実際に効く打鍵が食い違う。
    if (!event.metaKey || event.altKey || event.ctrlKey) return

    event.preventDefault()
    // 確定の打鍵は二つ。⌘Enter は作成のみで現在地を変えず、⌘⇧Enter は作成に加えて
    // 現在地をその第 1 ステップへ置く。
    void confirmCreation(event.shiftKey)
  }

  /**
   * 開示面の打鍵。**IME ガードは呼び出し元が既に通している** (AD-6)。
   *
   * # なぜ確定が素の Enter なのか
   *
   * この面に入力欄は無く、Enter が改行を意味する場所が存在しない。修飾キーを要求すれば、
   * 行を選んでから確定するまでの打鍵が一つ増えるだけである。
   *
   * # ⌘Enter に意味を与えない
   *
   * 既定表示の ⌘Enter は「完了して切り替え」である。同じ打鍵をこの面で受け付ければ、
   * 一覧から完了を宣言できることになり、**完了に一切触れない**という約束が破れる
   * (FR-4 / AD-2)。修飾キーを伴う Enter はすべて捨てる。
   */
  function onDisclosureKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      // 確定の途中では出ない。失敗したときの理由を読む機会が消える。
      if (selecting) return
      event.preventDefault()
      void leaveDisclosure()
      return
    }

    // 選んでいる行の**タスク**を直す (CAP-5)。**現在地は動かない** — 直す行為は
    // 位置にも完了にも触れない。
    if (isSurfaceKey(event, 'e')) {
      event.preventDefault()
      if (selecting || taskToEdit === null) return
      void openEdit(taskToEdit)
      return
    }

    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      if (selecting) return
      // 既定の動作 (面ごとのスクロール) を止める。入力位置の移動が選んだ行を
      // 見える位置まで送るため、二重に動かすと行を追い越す。
      event.preventDefault()
      void moveSelection(event.key === 'ArrowDown' ? 1 : -1)
      return
    }

    if (event.key !== 'Enter') return
    // 押しっぱなしの自動反復を確定として扱わない (既定表示と同じ理由)。
    if (event.repeat) return
    if (event.metaKey || event.shiftKey || event.altKey || event.ctrlKey) return

    event.preventDefault()
    void confirmSelection()
  }

  /**
   * 修正の面の打鍵。**IME ガードは呼び出し元が既に通している** (AD-6)。
   *
   * # なぜ確定が素の Enter ではないのか
   *
   * 追記の欄は 1 行 = 1 ステップであり、複数行を打つことが前提である (作成の面と同じ)。
   * 素の Enter を確定にすると、2 行目を打とうとした打鍵がそのまま保存になる。
   *
   * # ⌘⇧Enter に意味を与えない
   *
   * 作成の面の ⌘⇧Enter は「作成して着手」である。同じ打鍵をここで受け付ければ、直した
   * ついでに現在地が動くことになり、**修正は現在地に触れない**という約束が破れる。
   * 修飾キーを伴う Enter のうち効くのは ⌘Enter だけである。
   */
  function onEditKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      // 確定の途中では出ない。失敗したときの理由を読む機会が消える。
      if (editingTask) return
      event.preventDefault()
      // 下書きは確定されない。永続化もされない (AD-5)。
      void leaveEdit()
      return
    }

    if (event.key !== 'Enter') return
    // 押しっぱなしの自動反復を確定として扱わない (他の面と同じ理由)。
    if (event.repeat) return
    // 素の Enter と Shift+Enter は改行である。Option / Control は受け付けない。
    if (!event.metaKey || event.shiftKey || event.altKey || event.ctrlKey) return

    event.preventDefault()
    void confirmEdit()
  }

  onMount(() => {
    void refresh()

    const unlistenFocus = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) {
        // 新しい呼び出しである。前回の報せを畳み、完全なスナップショットを
        // 取り直す (AD-3 鮮度規則)。下書きもここで捨てられる。
        //
        // **作成の面も畳む。** 初期表示に現れてよい面ではなく (FR-2 / AD-15)、
        // 入力途中の題名とステップはフォーカス喪失で失われてよい (AD-2)。
        // **開示面も畳む。** 初期表示に現れてよい面ではなく (FR-19)、一覧は隠れている
        // 間持ち越さない。
        dismissNotices()
        discardTaskDraft()
        discardDisclosure()
        discardTaskEdit()
        void refresh()
      } else {
        // 呼び出して使ったら消える一時的な面として扱う (AD-15)。
        // Esc と同じ経路を通す。
        //
        // **ここで下書きを捨てる。** 取得時にだけ捨てていると、フォーカスのイベントを
        // 伴わない再表示が、作成の面を出したままの初期表示になる (FR-2 が禁じる)。
        discardTaskDraft()
        discardDisclosure()
        discardTaskEdit()
        void close()
      }
    })

    // イベントは再描画の契機にすぎず、状態の出所ではない (AD-3)。ペイロードを読まず、
    // 必ずスナップショットを取り直す。取りこぼしても次回の表示で正しくなる。
    // **利用者が起こした取り直しではないため、入力途中の下書きには触れない。**
    const unlistenPosition = listen(CURRENT_POSITION_CHANGED, () => {
      void refresh({ keepDirtyDraft: true })
    })

    return () => {
      void unlistenFocus.then((stop) => stop())
      void unlistenPosition.then((stop) => stop())
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

  <!--
    完了を宣言したことを述べる唯一の場所。**面の外に出す** — 宣言の直後に開くのは
    開示面であり、既定表示の側に置くと、最も述べる必要のある結末が面ごと隠れる
    (休息の報せと同じ理由)。
  -->
  {#if completionNotice}
    <p class="alert" role="alert">{completionNotice}</p>
  {/if}

  <!--
    既定表示は次の一手のみ (FR-2)。二つ以上のステップ内容も、タスク名も出さない。
    位置情報「第 N ステップ / 全 M ステップ」がこの制限の唯一の例外である。
  -->
  {#if creating}
    <!--
      作成の面 (CAP-4)。**既存のタスクを並べない。** 二つ以上のタスク名も、二つ以上の
      既存ステップの内容も同時に現れない — 選ばせる面を作った瞬間、それは CAP-9 の
      開示面そのものになる (FR-19)。ここで扱うのは「新しく書き留める」ことだけである。

      コアが読めていないときもこの面は出る。作成は失敗し、その理由が下に出る
      (I/O マトリクス「コア不在」)。
    -->
    <input
      class="title"
      type="text"
      bind:this={titleInput}
      bind:value={titleDraft}
      spellcheck="false"
      placeholder="タスクの題名"
      aria-label="タスクの題名"
    />

    <!--
      1 行 = 1 ステップ。行の順がそのまま連番 1..N になる。前後の空白を除いて空になる
      行は落ちる — 判定はコマンド境界の純粋関数が持つ。
    -->
    <textarea
      class="steps"
      bind:value={stepsDraft}
      rows="3"
      spellcheck="false"
      placeholder="1 行 = 1 ステップ"
      aria-label="ステップ (1 行 = 1 ステップ)"
    ></textarea>

    {#if createError}
      <p class="alert" role="alert">
        タスクを作成できなかった。何も保存されていない。
        <span class="detail">{createError}</span>
      </p>
    {/if}
  {:else if editing}
    <!--
      修正の面 (CAP-5 / FR-4 / FR-5)。**明示的な打鍵 (⌘E) かボタンを経てのみここに来る。**

      **一度に一つのタスクしか出さない。** 直す相手を選ばせる面は作らない — 選ばせた
      瞬間、それは CAP-9 の開示面そのものになる (作成の面と同じ排他である)。

      **完了の印も中断メモも出さない。** 直せるのは題名と本文だけであり、コマンドが
      運ぶ欄もそれだけである。進捗率・件数・総数も置かない (AD-15)。

      コアが読めていないときもこの面は出る。保存は失敗し、その理由が下に出る
      (I/O マトリクス「コア不在」)。
    -->
    {#if stateError}
      <p class="alert" role="alert">
        {stateError}
        <span class="detail">この面からタスクを直すことはできない。</span>
      </p>
    {/if}

    {#if editTaskId === null}
      <!-- 直す相手が無い。**確定の手段そのものを出さない。** -->
      <p class="empty">直せるタスクが無い。</p>
    {:else}
      <input
        class="title"
        type="text"
        bind:this={editTitleInput}
        bind:value={editTitleDraft}
        spellcheck="false"
        placeholder="タスクの題名"
        aria-label="タスクの題名"
      />

      <!--
        既存のステップは 1 個 = 1 欄である。**行ではなく欄で分ける** — 欄ごとに ID を
        持たせることが、1 行ずれても別のステップが書き換わらないための形である (FR-5)。

        **消す手がかりを置かない。** v1 に削除の経路は無く、空にした欄は保存時に
        拒まれる (コマンド境界の `STEP_CONTENT_MISSING`)。
      -->
      {#each editStepDrafts as step, index (step.stepId)}
        <input
          class="step-edit"
          type="text"
          data-step-id={step.stepId}
          bind:value={editStepDrafts[index].content}
          spellcheck="false"
          aria-label={`第 ${index + 1} ステップ`}
        />
      {/each}

      <!--
        末尾への追記。1 行 = 1 ステップであり、切り分けるのはコマンド境界の純粋関数で
        ある (作成と同じ規則)。空でよい。
      -->
      <textarea
        class="steps"
        bind:value={editAddedDraft}
        rows="2"
        spellcheck="false"
        placeholder="末尾に追記 (1 行 = 1 ステップ・省略可)"
        aria-label="追記するステップ (1 行 = 1 ステップ)"
      ></textarea>
    {/if}

    {#if editError}
      <p class="alert" role="alert">
        タスクを直せなかった。何も保存されていない。
        <span class="detail">{editError}</span>
      </p>
    {/if}
  {:else if disclosing}
    <!--
      開示面 (CAP-9 / FR-19)。**明示的な打鍵を経てのみここに来る。**

      並べ替え・改名・削除・一括編集の手がかりを一つも置かない (SPEC 非目標 / AD-15)。
      進捗率・件数・総数も置かない — 境界に運ぶ欄が無いため、描ける値がそもそも無い。

      コアが読めていないときもこの面は出る。移動はできず、その理由が上に出る
      (I/O マトリクス「コア不在」)。
    -->
    {#if stateError}
      <p class="alert" role="alert">
        {stateError}
        <span class="detail">この面から現在地を移すことはできない。</span>
      </p>
    {/if}
    {#if disclosureError}
      <p class="alert" role="alert">
        一覧を取得できなかった。現在地は変わっていない。
        <span class="detail">{disclosureError}</span>
      </p>
    {/if}
    {#if selectError}
      <p class="alert" role="alert">
        現在地を移せなかった。状態は変わっていない。
        <span class="detail">{selectError}</span>
      </p>
    {/if}

    <!--
      見出しとステップを一つの流れで見せる。**見出しは常にすべて現れ、ステップは一度に
      一つのタスクの分だけ現れる** (spec Boundaries / 設計上の賭け #1)。窓は固定寸法で
      あり、溢れた分は `main` のスクロールで最後の行まで到達できる。
    -->
    <div class="disclosure" bind:this={listElement}>
      {#each rows as row}
        {#if row.kind === 'task'}
          <!--
            見出しも選べる行である。**確定するとそのタスクが開き、直前に開いていた
            タスクが閉じる** — 現在地は動かず、切り替え履歴も増えない。

            開いているかどうかは `aria-expanded` が伝える。**開閉に印を与えない** —
            与えれば、下の現在地の印と並んで二つの三角が別の意味を持つことになる。

            **現在地の印は、そのタスクが閉じていても付く。** ステップの行だけが現在地を
            示す形にすると、別のタスクを開いた瞬間に現在地がどこにも現れない (spec
            Boundaries「現在地が指す行がどれか分かること」)。これは位置情報であって
            進捗の可視化ではない (AD-15 の「第 N / 全 M」と同じ類) — **件数も総数も
            割合も色も持たない、書体上の素朴な印だけである**。

            **ステップの行の `▸` とは別の字にする。** 三角は開閉の記号として読まれ、
            見出しの上では `aria-expanded` と衝突する。`aria-current` の値も
            `step` と `location` で分かれており、印と綴りの両方で区別が付く。

            click は選ぶだけで確定しない (ステップの行と同じ理由)。
          -->
          <button
            type="button"
            class="task-heading"
            class:selected={rowKey(row) === selectedRowKey}
            data-row-key={rowKey(row)}
            tabindex={rowKey(row) === selectedRowKey ? 0 : -1}
            aria-expanded={row.open}
            aria-current={row.holdsCurrentPosition ? 'location' : undefined}
            onclick={() => selectRow(rowKey(row))}
          >
            <span class="here">{row.holdsCurrentPosition ? '●' : ''}</span>
            <span class="content">{row.title}</span>
          </button>
        {:else}
          <!--
            **現在地**を移せるのはステップだけである。`aria-current="step"` が現在地の
            行を、印が完了を伝える — **色・バー・パーセントのいずれも用いない** (AD-15)。

            click は**選ぶだけで確定しない**。押した瞬間に現在地が動く形にすると、
            Space や修飾キーを伴う click までボタンの既定動作として確定に化け、Enter の
            分岐が意図して拒んでいる組み合わせがマウス経由で通る。
          -->
          <button
            type="button"
            class="step-row"
            class:selected={rowKey(row) === selectedRowKey}
            data-step-id={row.stepId}
            data-row-key={rowKey(row)}
            tabindex={rowKey(row) === selectedRowKey ? 0 : -1}
            aria-current={row.current ? 'step' : undefined}
            onclick={() => selectRow(rowKey(row))}
          >
            <span class="mark">{row.completed ? '✓' : ''}</span>
            <span class="here">{row.current ? '▸' : ''}</span>
            <span class="content">{row.content}</span>
          </button>
        {/if}
      {/each}
    </div>

    {#if rows.length === 0 && !stateError && !disclosureError && !snapshotError}
      <!-- 空欄にしない (I/O マトリクス「タスクが無い」)。 -->
      <p class="empty">まだタスクが無い。</p>
    {/if}
  {:else if stateError}
    <!--
      コアが読めていない。**未着手と取り違えない** — 取り違えれば、現在地が失われた
      ことが「まだ始めていない」として静かに描かれる。
    -->
    <p class="alert" role="alert">
      {stateError}
      <span class="detail">メニューバー項目からログの場所を確認すること。</span>
    </p>
  {:else if resting}
    <!--
      休息中 (CAP-10 / FR-15)。**現在地は値を保ったまま非活性であり、計時は止まって
      いる。** 残り時間も、休んでいる長さも出さない — 休息に締め切りを与えれば、
      それは休息ではなくなる (spec Never / AD-15)。

      **次の一手をここに出さない。** 出せば、休息中に次の作業が視界へ入る。戻る先が
      あることだけを述べ、中身は終了を宣言してから CAP-8 の形式で示す。
    -->
    <p class="next-action">休息中。</p>
    <p class="position">現在地は保たれている。連続作業時間は数えていない。</p>
  {:else if stepContent === null}
    {#if !snapshotError}
      <p class="next-action">まだ現在地が無い — 未着手である。</p>
    {/if}
  {:else}
    <p class="next-action">{stepContent}</p>
    <p class="position">第 {stepOrdinal} ステップ / 全 {stepCount} ステップ</p>

    <!--
      入力欄は既存の中断メモで初期化し、カーソルを末尾に置く (FR-7)。同じ欄が
      FR-8 の「記録済みの中断メモが他の画面を参照せず読める位置にある」も満たす。
    -->
    <textarea
      class="note"
      bind:this={noteInput}
      bind:value={noteDraft}
      rows="2"
      spellcheck="false"
      placeholder="中断メモ (省略可)"
      aria-label="中断メモ"
    ></textarea>

    <!--
      報せはフォーカスが入力欄にあるまま現れる。`role="alert"` が無ければ支援技術に
      何も伝わらない — この二つは、オーバーレイが開いたままになる唯一の経路であり、
      利用者が受け取る唯一の手がかりである。
    -->
    {#if noDestination}
      <p class="alert" role="alert">{noDestinationNotice}</p>
    {/if}
    {#if switchError}
      <p class="alert" role="alert">
        切り替えを確定できなかった。状態は変わっていない。
        <span class="detail">{switchError}</span>
      </p>
    {/if}
  {/if}

  <!--
    作成が確定したことを述べる唯一の場所。面は成功と同時に畳まれるため、ここに無ければ
    「作成できたのか」を確かめる方法が残らない。件数も登録数も述べない (AD-15 / SM-C1)。
  -->
  {#if !creating && !disclosing && !editing && creationNotice}
    <p class="alert" role="alert">{creationNotice}</p>
  {/if}

  <!--
    一覧から選んだのに何も書かれなかったことを述べる唯一の場所。**移った場合は述べない**
    — 既定表示がそのステップを示していることが結末そのものである。
  -->
  {#if !creating && !disclosing && !editing && selectionNotice}
    <p class="alert" role="alert">{selectionNotice}</p>
  {/if}

  <!--
    休息の終了について述べる唯一の場所。**休息中の面の中ではない** — 宣言に成功すれば
    面は畳まれるため、そこに置くと「休息中ではなかった」が誰にも届かない。
  -->
  {#if !creating && !disclosing && !editing && restNotice}
    <p class="alert" role="alert">
      {restNotice}
      {#if restErrorDetail}<span class="detail">{restErrorDetail}</span>{/if}
    </p>
  {/if}

  <!--
    修正が確定したことを述べる唯一の場所。面は成功と同時に畳まれるため、ここに無ければ
    「直せたのか」を確かめる方法が残らない。
  -->
  {#if !creating && !disclosing && !editing && edited}
    <p class="alert" role="alert">タスクを直した。現在地も完了も変えていない。</p>
  {/if}

  <!--
    マウスで到達できる操作 (I/O マトリクス「ボタン」)。**案内の打鍵と同じものしか
    置かない** — 打鍵に無い操作をボタンだけが持てば、面ごとに二つの操作体系が並ぶ。

    `onmousedown` で既定動作を止めるのは、入力位置を奪わないためである。中断メモを
    書きかけたまま押したボタンが欄からフォーカスを外すと、続きが打てなくなる。
  -->
  <div class="actions">
    {#if creating}
      <button type="button" onmousedown={preventStealingFocus} onclick={() => confirmCreation(false)}>作成</button>
      <button type="button" onmousedown={preventStealingFocus} onclick={() => confirmCreation(true)}>作成して着手</button>
      <button type="button" onmousedown={preventStealingFocus} onclick={() => leaveCreation()}>戻る</button>
    {:else if editing}
      {#if editTaskId !== null}
        <button type="button" onmousedown={preventStealingFocus} onclick={() => confirmEdit()}>保存</button>
      {/if}
      <button type="button" onmousedown={preventStealingFocus} onclick={() => leaveEdit()}>戻る</button>
    {:else if disclosing}
      {#if selectedRow}
        <button type="button" onmousedown={preventStealingFocus} onclick={() => confirmSelection()}>
          {selectedRow.kind === 'task' ? 'このタスクのステップを見る' : 'ここへ現在地を移す'}
        </button>
      {/if}
      {#if taskToEdit !== null}
        <button type="button" onmousedown={preventStealingFocus} onclick={() => openEdit(taskToEdit)}>このタスクを直す</button>
      {/if}
      <button type="button" onmousedown={preventStealingFocus} onclick={() => leaveDisclosure()}>戻る</button>
    {:else}
      {#if resting}
        <button type="button" onmousedown={preventStealingFocus} onclick={() => declareEndOfRest()}>休息を終える</button>
      {:else if stepContent !== null}
        <button type="button" onmousedown={preventStealingFocus} onclick={() => confirmSwitch()}>切り替え</button>
        <button type="button" onmousedown={preventStealingFocus} onclick={() => completeAndChooseNext()}>完了して次を選ぶ</button>
        <button type="button" onmousedown={preventStealingFocus} onclick={() => declareRest()}>休息に入る</button>
        <button type="button" onmousedown={preventStealingFocus} onclick={() => openEdit(null)}>このタスクを直す</button>
      {/if}
      <button type="button" onmousedown={preventStealingFocus} onclick={() => openCreation()}>新しいタスク</button>
      <button type="button" onmousedown={preventStealingFocus} onclick={() => openDisclosure()}>一覧</button>
      <button type="button" onmousedown={preventStealingFocus} onclick={() => close()}>閉じる</button>
    {/if}
  </div>

  {#if creating}
    <!--
      案内と実際に効く打鍵が食い違ってはならない。素の Enter と Shift+Enter は改行で
      あり、確定は ⌘ を伴う二つの打鍵だけである。
    -->
    <p class="hint">
      ⌘Enter で作成 · ⌘⇧Enter で作成して着手 · Enter で改行 · Esc で戻る
    </p>
  {:else if editing}
    <!--
      案内と実際に効く打鍵が食い違ってはならない。相手が無ければ保存の打鍵も効かない。
    -->
    <p class="hint">
      {#if editTaskId !== null}⌘Enter で保存 · Enter で改行 · {/if}Esc で戻る
    </p>
  {:else if disclosing}
    <!--
      案内と実際に効く打鍵が食い違ってはならない。この面に完了の宣言は無く、⌘Enter は
      どこにも束縛されていない。
    -->
    <!--
      **効かない打鍵を案内しない。** 選べる行が一つも無ければ ↑↓ も Enter も何も
      起こさない (一覧が空のとき・コアが読めていないとき)。

      **Enter の意味は行の種類で変わる。** 見出しの上で「現在地を移す」と案内すれば、
      案内と実際に効く打鍵が食い違う。
    -->
    <p class="hint">
      {#if rowKeys.length > 0}↑↓ で移動 · {selectedRow?.kind === 'task'
          ? 'Enter でこのタスクのステップを見る'
          : 'Enter でここへ現在地を移す'} · {/if}{#if taskToEdit !== null}⌘E でこのタスクを直す
        · {/if}Esc で戻る
    </p>
  {:else}
    <!-- 案内の語を行で割らない。割ると表示に改行が混じる。 -->
    <!--
      **休息中は案内が入れ替わる。** 切り替えも完了の宣言もこの面には無く、Enter は
      休息の終了の宣言だけを意味する (CAP-10)。案内と実際に効く打鍵を食い違わせない。
    -->
    <p class="hint">
      {#if resting}Enter で休息を終える · {:else if stepContent !== null}Enter で切り替え · ⌘Enter で完了して次を選ぶ · ⌘R で休息に入る · ⌘E でこのタスクを直す · {/if}⌘N で新しいタスク · ⌘L で一覧 · Esc で閉じる{#if hotkey && hotkey.registered} · {hotkey.accelerator} で開閉{/if} · 開く・終了はメニューバー項目からも
    </p>
  {/if}
</main>

<style>
  main {
    height: 100%;
    display: flex;
    flex-direction: column;
    /*
      ウィンドウは固定サイズであり、body は overflow: hidden である。常駐プロセスの
      状態を伝える警告 (スナップショットの失敗・ホットキーの登録失敗) は面の上に積まれる
      ため、作成の面と同時に出ると高さの見積りを超えうる。**切り取るのではなく辿れる
      ようにする。** `safe` は、はみ出したときに中央寄せをやめて先頭を見せる — これが
      無いと、溢れた分が上端の外へ出てスクロールでも到達できなくなる。
    */
    justify-content: safe center;
    overflow-y: auto;
    gap: 0.75rem;
    padding: 1.75rem 2rem;
  }

  /* 溢れたときに縮めて中身を切らない。縮む代わりに面ごとスクロールさせる。 */
  main > :global(*) {
    flex-shrink: 0;
  }

  .next-action {
    margin: 0;
    font-size: 1.15rem;
    line-height: 1.6;
  }

  /*
    位置情報である。進捗の可視化ではない (AD-15) — バー・パーセント・色による
    強調をここに足さない。
  */
  .position {
    margin: 0;
    font-size: 0.82rem;
    color: var(--overlay-muted);
  }

  /*
    マウスで到達できる操作の並び。**案内の打鍵と同じものしか並ばない。**

    溢れたら折り返す — 窓は固定寸法であり、切り取れば到達できない操作が生まれる。
  */
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }

  .actions button {
    margin: 0;
    padding: 0.25rem 0.6rem;
    border: 1px solid #3a3f47;
    border-radius: 4px;
    background: var(--overlay-raised);
    color: var(--overlay-fg);
    font: inherit;
    font-size: 0.78rem;
    line-height: 1.5;
    cursor: pointer;
  }

  .actions button:focus {
    outline: 1px solid var(--overlay-focus);
    outline-offset: 0;
  }

  /*
    入力欄は三つとも同じ器である (中断メモ・タスクの題名・ステップ)。
    `user-select` の解除は app.css が input / textarea に一括で掛けている — 面を
    足すたびに書き忘れうる規則を、面の側に置かないため。
  */
  .note,
  .title,
  .steps,
  .step-edit {
    margin: 0;
    width: 100%;
    resize: none;
    font: inherit;
    font-size: 0.92rem;
    line-height: 1.5;
    padding: 0.4rem 0.55rem;
    border: 1px solid #3a3f47;
    border-radius: 4px;
    background: var(--overlay-raised);
    color: var(--overlay-fg);
    cursor: text;
  }

  .note::placeholder,
  .title::placeholder,
  .steps::placeholder,
  .step-edit::placeholder {
    color: var(--overlay-muted);
  }

  .note:focus,
  .title:focus,
  .steps:focus,
  .step-edit:focus {
    outline: 1px solid var(--overlay-focus);
    outline-offset: 0;
  }

  /*
    開示面の一覧 (CAP-9)。**溢れたら切り取るのではなく辿れるようにする** — 高さを
    固定せず、`main` が唯一のスクロール容器であり続ける。器を入れ子にしてそちらにも
    スクロールを持たせると、どちらが動くかが行の位置で変わる。
  */
  .disclosure {
    display: flex;
    flex-direction: column;
    width: 100%;
    gap: 0.1rem;
  }

  /*
    タスクの見出し。**選べる行である** — 確定するとそのタスクのステップが並ぶ。
    ステップの行より小さく淡いままにして、見出しと行の区別を保つ。
  */
  .task-heading {
    display: flex;
    align-items: baseline;
    gap: 0.3rem;
    width: 100%;
    margin: 0.5rem 0 0.15rem;
    padding: 0.22rem 0.4rem;
    border: 1px solid transparent;
    border-radius: 4px;
    background: none;
    font: inherit;
    font-size: 0.8rem;
    line-height: 1.5;
    color: var(--overlay-muted);
    text-align: left;
    /* click は選ぶだけで確定しない。それでも押せる行であることは示す。 */
    cursor: pointer;
  }

  .task-heading:first-child {
    margin-top: 0;
  }

  /* 選んでいる行と、入力位置がある行を描き分ける (`.step-row` と同じ規則)。 */
  .task-heading.selected {
    background: var(--overlay-raised);
  }

  .task-heading:focus {
    outline: 1px solid var(--overlay-focus);
    outline-offset: 0;
  }

  .step-row {
    display: flex;
    align-items: baseline;
    gap: 0.3rem;
    width: 100%;
    margin: 0;
    padding: 0.22rem 0.4rem;
    border: 1px solid transparent;
    border-radius: 4px;
    background: none;
    font: inherit;
    font-size: 0.92rem;
    line-height: 1.5;
    color: var(--overlay-fg);
    text-align: left;
    /* click は選ぶだけで確定しない。それでも押せる行であることは示す。 */
    cursor: pointer;
  }

  /*
    **選んでいる行と、入力位置がある行を描き分ける。** 同じ描き方にすると、フォーカスが
    面の外へ出たときに選択だけが残っている状態と見分けが付かない。どちらも
    **選択の表示であって進捗の表示ではない** — 完了の有無で色を変えない (AD-15)。
  */
  .step-row.selected {
    background: var(--overlay-raised);
  }

  /* 入力欄の `:focus` と同じ線である (`.note:focus` ほか)。 */
  .step-row:focus {
    outline: 1px solid var(--overlay-focus);
    outline-offset: 0;
  }

  /*
    完了と現在地の印。**書体上の素朴な印だけである** — バーもパーセントも色分けも
    持たない (AD-15)。幅を固定して、印の無い行と内容の字が揃うようにする。
  */
  .mark,
  .here {
    flex: 0 0 0.9rem;
    text-align: center;
    color: inherit;
  }

  .content {
    flex: 1 1 auto;
  }

  /* タスクが 1 個も無いときの 1 行。空欄にしない。 */
  .empty {
    margin: 0;
    font-size: 0.92rem;
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
