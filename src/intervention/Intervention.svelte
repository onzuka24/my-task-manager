<script lang="ts">
  /**
   * 介入パネル (CAP-10 / FR-15)。**入力先を奪わない非活性パネルの中身である** (AD-6)。
   *
   * # ここに入力欄を置かない
   *
   * 日本語入力を阻害する上流の既知問題はウィンドウレベルの高いパネルに掛かる。
   * **テキスト入力を伴う面をパネルとして実装してはならない** (AD-6)。応答は二択だけで
   * あり、打鍵はグローバルホットキー、あるいはクリックで受ける。
   *
   * # 経過時間も残り時間も描かない
   *
   * 境界が運ぶ欄に無い (`InterventionSnapshot`)。**カウントダウンを常時表示しない**
   * (spec Never) のは、数字が進捗の可視化と同じ種類の圧力になるためである (AD-15)。
   *
   * # 強さは「消えないこと」で表す
   *
   * FR-15 が求めるのは「いずれかが選ばれるまで消えず、単純な無視によって解消されない」
   * ことであって、視界を占有することではない。Esc も閉じるボタンも持たない —
   * **この面に「閉じる」という操作は存在しない。**
   */
  import { onMount } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { listen } from '@tauri-apps/api/event'

  /** src-tauri/src/adapters/hotkey/mod.rs の `ResponseHotkeyStatus` と 1:1。 */
  type ResponseHotkeyStatus = {
    restAccelerator: string
    graceAccelerator: string
    registered: boolean
    error: string | null
  }

  /**
   * src-tauri/src/commands/mod.rs の `InterventionSnapshot` と 1:1。
   *
   * `invoke<T>` は実行時検査を行わないため、片方だけ変えても警告は出ない。Rust 側の
   * `tests::the_intervention_snapshot_keeps_its_wire_contract` が形を固定している。
   */
  type InterventionSnapshot = {
    hotkey: ResponseHotkeyStatus | null
    stateError: string | null
    shown: boolean
  }

  /** src-tauri/src/commands/mod.rs の `AnswerInterventionRequest` と 1:1。 */
  type InterventionChoice = 'rest' | 'grace'

  type AnswerInterventionRequest = {
    choice: InterventionChoice
  }

  type AnswerInterventionOutcome = {
    answered: boolean
  }

  /** コアから届く、再描画の契機 (AD-3)。状態は運ばれてこない。 */
  const INTERVENTION_RAISED = 'intervention_raised'

  /** スナップショット取得の再試行回数と間隔。Overlay.svelte と同じ規則である。 */
  const SNAPSHOT_RETRIES = 10
  const SNAPSHOT_RETRY_INTERVAL_MS = 100

  let hotkey = $state<ResponseHotkeyStatus | null>(null)
  let stateError = $state<string | null>(null)
  let snapshotError = $state<string | null>(null)
  let answerError = $state<string | null>(null)
  /** 応答の要求が飛んでいる間。**二重確定を防ぐ。** */
  let answering = $state(false)

  /**
   * 応答の打鍵を案内してよいか。
   *
   * **登録できていないホットキーを案内しない。** 案内すれば、押しても効かない打鍵を
   * 示したまま「選ぶまで消えない」面が残る。登録の失敗はクリックでの応答へ倒す
   * (AD-7)。
   */
  const hotkeysUsable = $derived(hotkey?.registered === true)

  function sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }

  /**
   * 完全なスナップショットを取り直す (AD-3 鮮度規則)。
   *
   * **隠れている間に受け取ったイベントに依存しない。** パネルは起動時に生成されて
   * 隠されており、介入が発せられるまで一度も見えない。
   */
  async function refresh(): Promise<void> {
    for (let attempt = 0; attempt < SNAPSHOT_RETRIES; attempt += 1) {
      try {
        const snapshot = await invoke<InterventionSnapshot>('get_intervention_snapshot')
        hotkey = snapshot.hotkey
        stateError = snapshot.stateError
        snapshotError = null
        return
      } catch (error) {
        if (attempt === SNAPSHOT_RETRIES - 1) {
          console.error('failed to fetch the intervention snapshot', error)
          // **空白のまま出さない。** 出れば、消えない面が理由も示さずに居座る。
          snapshotError = String(error)
          hotkey = null
          return
        }
        await sleep(SNAPSHOT_RETRY_INTERVAL_MS)
      }
    }
  }

  /**
   * 二択のどちらかを確定する (FR-15)。
   *
   * **閉じるのはコアである。** ここは要求を出すだけであり、パネルを自ら隠さない —
   * 介入を閉じうる経路が二つあってはならない (AD-7)。
   */
  async function answer(choice: InterventionChoice): Promise<void> {
    if (answering) return
    answering = true
    answerError = null
    const request: AnswerInterventionRequest = { choice }
    try {
      const outcome = await invoke<AnswerInterventionOutcome>('answer_intervention', { request })
      if (!outcome.answered) {
        // ホットキーとクリックが同時に届いた二つ目がこれである。**現在地は二度
        // 動いていない。** パネルは既に引っ込む途中であり、述べることは無い。
        console.info('the intervention had already been answered')
      }
    } catch (error) {
      console.error('failed to answer the intervention', error)
      // **面は消えない。** 応答が確定していない以上、消してはならない。
      answerError = String(error)
    } finally {
      answering = false
    }
  }

  onMount(() => {
    void refresh()

    // イベントは再描画の契機にすぎず、状態の出所ではない (AD-3)。ペイロードを読まず、
    // 必ずスナップショットを取り直す。
    const unlistenRaised = listen(INTERVENTION_RAISED, () => {
      answerError = null
      void refresh()
    })

    return () => {
      void unlistenRaised.then((stop) => stop())
    }
  })
</script>

<main>
  <!--
    **数字を書かない。** 閾値も猶予も設定で変わる (AD-11) うえ、境界はその値を運ばない
    — 運ばせれば、それは残り時間の常時表示への最初の一歩である (spec Never)。
  -->
  <p class="prompt">続けて作業している。休息の頃合いである。</p>

  {#if stateError}
    <p class="alert" role="alert">
      {stateError}
      <span class="detail">応答しても現在地は変えられない。</span>
    </p>
  {/if}
  {#if snapshotError}
    <p class="alert" role="alert">
      常駐プロセスの状態を取得できなかった。
      <span class="detail">{snapshotError}</span>
    </p>
  {/if}
  {#if answerError}
    <p class="alert" role="alert">
      応答を記録できなかった。状態は変わっていない。
      <span class="detail">{answerError}</span>
    </p>
  {/if}

  <!--
    二択。**これがすべてである** — 「無視する」も「閉じる」も無い (FR-15)。
    クリックだけで応答できる形を常に保つ。ホットキーが登録できていてもいなくても、
    この二つのボタンは同じように効く (AD-7)。
  -->
  <div class="choices">
    <button type="button" class="choice" disabled={answering} onclick={() => answer('rest')}>
      <span class="label">休息に入る</span>
      {#if hotkeysUsable}<span class="key">{hotkey?.restAccelerator}</span>{/if}
    </button>
    <button type="button" class="choice" disabled={answering} onclick={() => answer('grace')}>
      <span class="label">猶予の後に出直す</span>
      {#if hotkeysUsable}<span class="key">{hotkey?.graceAccelerator}</span>{/if}
    </button>
  </div>

  <!--
    **登録に失敗しても介入は出る** (AD-7)。そのことを理由とともに述べ、クリックで
    応答できると案内する。黙っていると、案内された打鍵が効かない面が残る。
  -->
  {#if hotkey && !hotkey.registered}
    <p class="hint">
      応答のホットキーを登録できなかった。クリックで選ぶこと。
      {#if hotkey.error}<span class="detail">{hotkey.error}</span>{/if}
    </p>
  {/if}
</main>

<style>
  /*
    **小さく、隅に、しかし答えるまで居続ける。** 強さは消えないことで表し、大きさでは
    表さない (spec Design Notes)。
  */
  main {
    height: 100%;
    display: flex;
    flex-direction: column;
    justify-content: safe center;
    overflow-y: auto;
    gap: 0.7rem;
    padding: 1rem 1.1rem;
  }

  main > :global(*) {
    flex-shrink: 0;
  }

  .prompt {
    margin: 0;
    font-size: 0.98rem;
    line-height: 1.5;
  }

  .choices {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .choice {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
    width: 100%;
    margin: 0;
    padding: 0.35rem 0.55rem;
    border: 1px solid #3a3f47;
    border-radius: 4px;
    background: var(--overlay-raised);
    font: inherit;
    font-size: 0.9rem;
    line-height: 1.4;
    color: var(--overlay-fg);
    text-align: left;
    cursor: pointer;
  }

  .choice:disabled {
    cursor: default;
    color: var(--overlay-muted);
  }

  .key {
    color: var(--overlay-muted);
    font-size: 0.74rem;
  }

  .hint {
    margin: 0;
    font-size: 0.74rem;
    color: var(--overlay-muted);
  }

  .alert {
    margin: 0;
    font-size: 0.8rem;
    line-height: 1.45;
    color: var(--overlay-alert);
  }

  .detail {
    display: block;
    color: var(--overlay-muted);
    font-size: 0.7rem;
  }
</style>
