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

  function sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }

  /** 表示できる現在地を持っていない状態へ戻す。 */
  function forgetPosition(): void {
    stepContent = null
    stepOrdinal = null
    stepCount = null
    notePrefill = ''
    noteDraft = ''
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

        const preserve = keepDirtyDraft && noteIsDirty
        notePrefill = snapshot.interruptionNote ?? ''
        if (!preserve) {
          // 記録済みのメモで初期化し直す。ここが FR-7 の「上書き前の内容の提示」で
          // あり、同時に「入力途中の離脱では下書きが破棄される」の実体でもある。
          noteDraft = notePrefill
          await focusNoteAtEnd()
        }
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
   * 切り替えを確定させる (CAP-7)。
   *
   * 離脱側のメモ確定・完了宣言 (任意)・現在地の移動・切り替え履歴の追記は、コア側の
   * **単一のトランザクション**で確定する (AD-5)。ここが行うのは要求と、その結末に
   * 応じた離脱だけである。
   */
  async function confirmSwitch(declareCompletion: boolean): Promise<void> {
    // 現在地が無ければ離れるべき場所も無い。二重確定も防ぐ。
    if (stepContent === null || switching) return
    switching = true
    dismissNotices()
    // **提示されたメモをそのまま送り返さない。** 送り返せば、読み返しただけの
    // 切り替えが「メモを書いた」として履歴に残り、SM-C3 の記入率が膨らむ。
    const request: SwitchRequest = {
      note: noteIsDirty ? noteDraft : null,
      declareCompletion,
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
        completionDeclared: declareCompletion,
      }
    } catch (error) {
      console.error('failed to commit the switch', error)
      switchError = String(error)
    } finally {
      switching = false
    }
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

    if (event.key === 'Escape') {
      // 確定の途中では閉じない。閉じてしまうと、失敗したときの理由を読む機会が
      // 画面ごと消える。
      if (switching) return
      event.preventDefault()
      // 下書きは確定されない。永続化もされない (AD-5)。
      void close()
      return
    }

    // 作成の面へ入る唯一の打鍵 (FR-2 / AD-15)。確定の途中では入らない — 入れば、
    // 失敗したときの理由を読む機会が面ごと消える。
    if (event.key === 'n' && event.metaKey && !event.shiftKey && !event.altKey && !event.ctrlKey) {
      if (switching) return
      event.preventDefault()
      void openCreation()
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
    // 完了の宣言は同じ一連の操作から 1 打鍵で到達する (FR-4)。修飾キーの有無だけが
    // 違い、宣言しない切り替えも同じく 1 打鍵である。
    void confirmSwitch(event.metaKey)
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

  onMount(() => {
    void refresh()

    const unlistenFocus = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) {
        // 新しい呼び出しである。前回の報せを畳み、完全なスナップショットを
        // 取り直す (AD-3 鮮度規則)。下書きもここで捨てられる。
        //
        // **作成の面も畳む。** 初期表示に現れてよい面ではなく (FR-2 / AD-15)、
        // 入力途中の題名とステップはフォーカス喪失で失われてよい (AD-2)。
        dismissNotices()
        discardTaskDraft()
        void refresh()
      } else {
        // 呼び出して使ったら消える一時的な面として扱う (AD-15)。
        // Esc と同じ経路を通す。
        //
        // **ここで下書きを捨てる。** 取得時にだけ捨てていると、フォーカスのイベントを
        // 伴わない再表示が、作成の面を出したままの初期表示になる (FR-2 が禁じる)。
        discardTaskDraft()
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
  {:else if stateError}
    <!--
      コアが読めていない。**未着手と取り違えない** — 取り違えれば、現在地が失われた
      ことが「まだ始めていない」として静かに描かれる。
    -->
    <p class="alert" role="alert">
      {stateError}
      <span class="detail">メニューバー項目からログの場所を確認すること。</span>
    </p>
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
  {#if !creating && creationNotice}
    <p class="alert" role="alert">{creationNotice}</p>
  {/if}

  {#if creating}
    <!--
      案内と実際に効く打鍵が食い違ってはならない。素の Enter と Shift+Enter は改行で
      あり、確定は ⌘ を伴う二つの打鍵だけである。
    -->
    <p class="hint">
      ⌘Enter で作成 · ⌘⇧Enter で作成して着手 · Enter で改行 · Esc で戻る
    </p>
  {:else}
    <!-- 案内の語を行で割らない。割ると表示に改行が混じる。 -->
    <p class="hint">
      {#if stepContent !== null}Enter で切り替え · ⌘Enter で完了して切り替え · {/if}⌘N で新しいタスク · Esc で閉じる{#if hotkey && hotkey.registered} · {hotkey.accelerator} で開閉{/if} · 終了はメニューバー項目から
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
    入力欄は三つとも同じ器である (中断メモ・タスクの題名・ステップ)。
    `user-select` の解除は app.css が input / textarea に一括で掛けている — 面を
    足すたびに書き忘れうる規則を、面の側に置かないため。
  */
  .note,
  .title,
  .steps {
    margin: 0;
    width: 100%;
    resize: none;
    font: inherit;
    font-size: 0.92rem;
    line-height: 1.5;
    padding: 0.4rem 0.55rem;
    border: 1px solid #3a3f47;
    border-radius: 4px;
    background: #1d2026;
    color: var(--overlay-fg);
    cursor: text;
  }

  .note::placeholder,
  .title::placeholder,
  .steps::placeholder {
    color: var(--overlay-muted);
  }

  .note:focus,
  .title:focus,
  .steps:focus {
    outline: 1px solid #5a6270;
    outline-offset: 0;
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
