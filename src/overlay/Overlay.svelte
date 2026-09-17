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
   * 入力欄へフォーカスを移し、カーソルを末尾に置く (FR-7)。
   *
   * 末尾に置くことが「追記の形を選べる」を成立させる。先頭や全選択にすると、
   * 一打鍵で既存のメモを消してしまう形になる。
   */
  async function focusNoteAtEnd(): Promise<void> {
    await tick()
    const input = noteInput
    if (!input) return
    input.focus()
    const end = input.value.length
    input.setSelectionRange(end, end)
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

    if (event.key === 'Escape') {
      // 確定の途中では閉じない。閉じてしまうと、失敗したときの理由を読む機会が
      // 画面ごと消える。
      if (switching) return
      event.preventDefault()
      // 下書きは確定されない。永続化もされない (AD-5)。
      void close()
      return
    }

    if (event.key !== 'Enter') return
    // Shift+Enter は改行。Option / Control を伴う Enter は受け付けない — 受け付けると
    // 案内 (⌘Enter) と実際に効く打鍵が食い違う。
    if (event.shiftKey || event.altKey || event.ctrlKey) return

    event.preventDefault()
    // 完了の宣言は同じ一連の操作から 1 打鍵で到達する (FR-4)。修飾キーの有無だけが
    // 違い、宣言しない切り替えも同じく 1 打鍵である。
    void confirmSwitch(event.metaKey)
  }

  onMount(() => {
    void refresh()

    const unlistenFocus = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) {
        // 新しい呼び出しである。前回の報せを畳み、完全なスナップショットを
        // 取り直す (AD-3 鮮度規則)。下書きもここで捨てられる。
        dismissNotices()
        void refresh()
      } else {
        // 呼び出して使ったら消える一時的な面として扱う (AD-15)。
        // Esc と同じ経路を通す。
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
  {#if stateError}
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

  <p class="hint">
    {#if stepContent !== null}Enter で切り替え · ⌘Enter で完了して切り替え · {/if}Esc
    で閉じる{#if hotkey && hotkey.registered} · {hotkey.accelerator} で開閉{/if} ·
    終了はメニューバー項目から
  </p>
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

  /*
    位置情報である。進捗の可視化ではない (AD-15) — バー・パーセント・色による
    強調をここに足さない。
  */
  .position {
    margin: 0;
    font-size: 0.82rem;
    color: var(--overlay-muted);
  }

  .note {
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
    /*
      app.css は body 全体に user-select: none を掛けている (器としてのオーバーレイ)。
      入力欄でだけ解除する — 解除しないと選択・カーソル移動が効かない。
    */
    user-select: text;
    -webkit-user-select: text;
    cursor: text;
  }

  .note::placeholder {
    color: var(--overlay-muted);
  }

  .note:focus {
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
