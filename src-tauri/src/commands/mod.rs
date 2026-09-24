//! Tauri command 境界 (AD-3)。
//!
//! フロントからコアへはコマンドのみ。コアからフロントへはイベントのみ。
//!
//! AD-3 の鮮度規則により、オーバーレイは**表示されるたびに**
//! [`get_overlay_snapshot`] で完全なスナップショットを取得してから描画する。
//! 隠れている間に受け取ったイベントに依存してはならない。
//!
//! 状態は `Builder::manage` で**起動前に**預ける。webview は常駐プロセスの `setup` が
//! 終わる前にもコマンドを呼びうるため、state そのものが未管理という状態を作らない。
//! 中身がまだ確定していないことは `Err` として表現し、フロントが再試行できるようにする。
//!
//! # コアの不在は既定値で埋めない
//!
//! [`Core`] は永続化の復元に成功したときにだけ `manage` される (`lib.rs`)。不在を既定値で
//! 埋めたコアに置き換えると、**現在地**が失われた事実が「未着手」として静かに上書きされ、
//! 次の書き込みで確定してしまう。[`tauri::Manager::try_state`] の `None` は、**状態を
//! 書き換える側**では明示的なエラーに、**表示する側**では [`OverlaySnapshot::state_error`]
//! に変換する。表示側でコマンドごと失敗させないのは、ホットキーの登録結果まで道連れに
//! なるためである — DB が開けずホットキーも死んでいるとき、唯一の呼び出し経路が死んで
//! いる事実が誰にも伝わらなくなる (spec Never「`hotkey` を失わせない」)。
//!
//! # 判断は純粋関数へ切り出す
//!
//! [`require_core`] と [`snapshot_of`] は `AppHandle` を取らない。呼び出し側に埋め込んだ
//! ままでは、生きた Tauri アプリを起動しない限り一行も検証できない
//! (`adapters/presentation` の `toggle_action` と同じ流儀)。
//!
//! # 中断メモの本文をログに書かない
//!
//! スパイン「一貫性の規約」。**切り替え**の成否は記録してよいが、本文は決して残さない。

use std::sync::Mutex;

use tauri::{AppHandle, Manager, Runtime, State};

use crate::adapters::hotkey::HotkeyStatus;
use crate::adapters::presentation;
use crate::domain::state::{Core, CoreState};
use crate::domain::task::{InterruptionNote, Step, StepId, Task, TaskId};

/// コアが `manage` されていないときに示す理由。
///
/// 利用者に示す文であり、ログではない。復元に失敗したことがログにしか出ないと、
/// **現在地**の喪失に気づく手段がログファイルを開くことだけになる。
const CORE_MISSING: &str =
    "保存された状態を読み込めていない。現在地と中断メモを表示できず、切り替えも記録できない。";

/// **`Core` の不在を明示的なエラーへ変換する純粋関数** (I/O マトリクス「コア不在」)。
///
/// **状態を書き換える経路が使う。** 表示する経路は [`snapshot_of`] を通り、コマンドごと
/// 失敗させずに [`OverlaySnapshot::state_error`] として運ぶ。
fn require_core<T>(core: Option<T>) -> Result<T, String> {
    core.ok_or_else(|| CORE_MISSING.to_string())
}

/// 常駐プロセスが保持する、ドメインに属さない起動時の状態。
#[derive(Default)]
pub struct ResidentStatus {
    hotkey: Mutex<Option<HotkeyStatus>>,
}

impl ResidentStatus {
    /// ホットキーの登録結果を確定させる。
    pub fn set_hotkey(&self, status: HotkeyStatus) {
        // 毒された Mutex でも常駐は止めない。ここに保持するのは起動時の事実であり、
        // 不変条件を壊しうる途中状態ではない。
        let mut slot = self
            .hotkey
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *slot = Some(status);
    }

    /// 確定済みのホットキーの登録結果。まだ確定していなければ `None`。
    pub fn hotkey(&self) -> Option<HotkeyStatus> {
        self.hotkey
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

/// オーバーレイが描画に必要とするすべて。
///
/// フィールド名は `src/overlay/Overlay.svelte` の `OverlaySnapshot` 型と 1:1 で
/// 対応する。`invoke<T>` は実行時検査を行わないため、ここを変えると警告が無言で
/// 出なくなる。契約は `tests::the_snapshot_keeps_its_wire_contract` で固定する。
///
/// # なぜ**次の一手**の型を作らないのか
///
/// 用語集は `NextAction` を「**現在地**が指す**ステップ**の表示上の呼称」と定め、
/// **v1 では型を持たない**と明記している (AD-10)。ここで入れ子の型を起こせば、
/// 「独立したエンティティとして実装してはならない」に反する。欄を平らに並べる。
///
/// # **切り替え履歴**に由来する欄が一つも無い
///
/// 履歴は SM-C3 の測定基盤であって表示物ではない (AD-15)。境界に欄が無いため、
/// フロントが描画しようとしても運ぶ値が存在しない。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySnapshot {
    /// ホットキーの登録結果。
    ///
    /// **コアが読めなくてもここは必ず埋まる。** ホットキーは唯一の呼び出し経路であり、
    /// それが死んでいることは他の失敗に巻き込まれて消えてはならない。
    pub hotkey: HotkeyStatus,
    /// コアが `manage` されていないときの理由。読めていれば `None`。
    ///
    /// **`stepContent` が `None` である理由を、未着手と区別するために要る。**
    /// 区別しなければ、状態を読めていないことが「未着手」として描かれる。
    pub state_error: Option<String>,
    /// **現在地**が指す**ステップ**の内容 — **次の一手**。**未着手**なら `None`。
    pub step_content: Option<String>,
    /// 「第 N ステップ」の N。**未着手**なら `None`。
    pub step_ordinal: Option<u32>,
    /// 「全 M ステップ」の M。**未着手**なら `None`。
    ///
    /// N と M は**位置情報**であって進捗の可視化ではない。比率を運ばないのは意図で
    /// ある (AD-15) — 割り算をフロントで起こさせないため、両方を数のまま渡す。
    pub step_count: Option<u32>,
    /// 記録済みの**中断メモ**。無ければ `None`。
    ///
    /// **入力欄の初期値でもある。** 既存のメモで初期化することが、FR-7 の
    /// 「上書き前の内容の提示」と「追記の形を選べる」を同時に満たす。
    pub interruption_note: Option<String>,
}

/// **コア状態を描画用のスナップショットへ落とす純粋関数。**
///
/// `state` が `None` なのはコアが `manage` されていない場合である (I/O マトリクス
/// 「コア不在」)。そのときも `hotkey` は必ず運ぶ。
///
/// # 引き当ては一度だけ
///
/// **ステップ**と、それを含む**タスク**を別々に引かない。二度引けば、二度目が外れた
/// ときに「内容は出ているが全体数だけ空」という組み合わせが生まれ、位置情報が
/// 「第 3 ステップ / 全 — ステップ」として描かれる。一度の引き当てから四つを同時に
/// 決めることで、その組み合わせを存在させない。
fn snapshot_of(hotkey: HotkeyStatus, state: Option<&CoreState>) -> OverlaySnapshot {
    let empty = OverlaySnapshot {
        hotkey,
        state_error: None,
        step_content: None,
        step_ordinal: None,
        step_count: None,
        interruption_note: None,
    };

    let Some(state) = state else {
        return OverlaySnapshot {
            state_error: Some(CORE_MISSING.to_string()),
            ..empty
        };
    };

    let shown = state.current_position().step_id().and_then(|step_id| {
        let task = state.task_of_step(step_id)?;
        Some((task.step(step_id)?, task.steps().len()))
    });
    // **未着手**。空欄にせず、そうと分かる形で返す (I/O マトリクス「未着手」)。
    let Some((step, count)) = shown else {
        return empty;
    };

    OverlaySnapshot {
        step_content: Some(step.content().to_string()),
        step_ordinal: Some(step.ordinal()),
        step_count: Some(u32::try_from(count).unwrap_or(u32::MAX)),
        interruption_note: step.interruption_note().map(|note| note.text().to_string()),
        ..empty
    }
}

/// **開示面**の一覧を成す 1 行 (CAP-9 / FR-19)。
///
/// **用途専用の平たい形である。** ドメインの [`Task`] / [`Step`] をそのまま線に乗せない —
/// 乗せれば `ordinal`・`completedAt`・`interruptionNote` が境界へ出て、この面に置かないと
/// 決めた値が「たまたま描いていないだけ」になる。運ぶのは描くものだけである。
///
/// # 見出しに**完了**の欄が無い
///
/// **タスク**は**完了**の状態を持たない (用語集: **完了**は**ステップ**に対して宣言
/// される)。変種を分けることで、見出しに意味の無い欄が生まれない。
///
/// # 見出しは常にすべて現れ、**ステップ**は一度に一つの**タスク**の分だけ現れる
///
/// 設計上の賭け #1 (一度に見せるのは次の一手だけ) に従い、**全部の**ステップ**が同時に
/// 見えてはならない** (spec Boundaries)。並べる**ステップ**を選ぶのはこの射影であり、
/// フロントの描画条件ではない — 描画側で濾す形にすると、線に乗った時点で全体像が既に
/// 渡っており、「見えていないだけ」になる。
///
/// # **現在地**は、その**タスク**が閉じていても分かる
///
/// spec Boundaries は「**現在地**が指す行がどれか分かること」を無条件に求める。**ステップ**
/// の行だけがそれを負うと、別の**タスク**を開いた瞬間に**現在地**がどこにも現れなくなる。
/// 見出しの側が「この**タスク**の中にある」を負い、**ステップ**の行が「この行である」を
/// 負う。どちらも**位置情報**であって進捗の可視化ではない — AD-15 が「第 N / 全 M」に
/// ついて認めているのと同じ類である。**件数・総数・割合・色のいずれも運ばない。**
///
/// # 件数・進捗率に由来する欄が一つも無い
///
/// 「全 M ステップ」のような総数も、完了の数も運ばない (AD-15 / spec Never)。欄が無ければ
/// フロントがどう書こうと描ける値が存在しない。契約は
/// [`tests::the_disclosure_rows_keep_their_wire_contract`] が固定する。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DisclosureRow {
    /// **タスク**の見出し。**タスク**が在る限り必ず現れる。
    ///
    /// 確定すると開く**タスク**がこれに移り、直前に開いていた**タスク**は閉じる。
    /// **確定しても**現在地**は動かず、**切り替え履歴**も増えない** (spec Boundaries)。
    #[serde(rename_all = "camelCase")]
    Task {
        /// 開く**タスク**を指定するときに送り返す ID ([`DisclosureRequest`])。
        task_id: String,
        /// **タスク**の題名。
        title: String,
        /// この**タスク**の**ステップ**が続いているか。
        ///
        /// **一覧で真になる見出しは高々一つである。** 開いた状態を積み上げられない
        /// ことが、一覧が全体像へ戻らないための条件である (spec Boundaries)。
        open: bool,
        /// **現在地**がこの**タスク**の中にあるか。
        ///
        /// **[`DisclosureRow::Step::current`] と別の欄である。** あちらは「この行が
        /// **現在地**である」、こちらは「**現在地**はこの**タスク**の中にある」。
        /// **[`DisclosureRow::Task::open`] とも別である** — 閉じている**タスク**が
        /// **現在地**を抱えていることこそ、この欄が要る理由である。一覧で真になる見出しは
        /// 高々一つであり (CAP-6 / FR-6)、**未着手**のときは一つも無い。
        holds_current_position: bool,
    },
    /// **ステップ**の行。
    #[serde(rename_all = "camelCase")]
    Step {
        /// 選択を確定するときに送り返す ID。**`ordinal` ではない** (FR-5)。
        step_id: String,
        /// **ステップ**の内容。
        content: String,
        /// **完了**しているか。**書体上の素朴な印**として描かれる (AD-15)。
        completed: bool,
        /// **現在地**が指す行か。
        current: bool,
    },
}

/// `get_disclosure_surface` が受け取る要求。**これがコマンドの引数型そのものである。**
///
/// フロントは `invoke('get_disclosure_surface', { request: { openTaskId } })` と呼ぶ。
/// [`SwitchRequest`] と同じ理由で名前付きの型として持つ — 平らな引数にすると、コマンドの
/// 仮引数名が唯一の契約になり、Tauri を起動せずに検証できる型が一つも残らない。
///
/// # なぜ開いている**タスク**をフロントが送るのか
///
/// 開閉は揮発ビュー状態であり、所有者は Svelte である (AD-2 の表「開示面の開閉状態」)。
/// **コアに新しい状態を足さない** — 足せば、閉じた時点で破棄されるはずのものが永続化の
/// 対象になる。コアが決めるのは「`null` のとき何が開くか」だけであり、それは**現在地**
/// から一意に定まる。
///
/// # なぜ知らない欄を拒むのか
///
/// 唯一の欄が `Option` であり、**綴りを取り違えても復元は成功する** — 欄が欠けたものと
/// して `None` になり、見出しを確定しても黙って**現在地**の**タスク**が開き続ける。
/// [`SelectStepRequest`] は必須の欄を持つため取り違えが `Err` として現れるが、こちらは
/// 拒まない限りどこにも現れない。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisclosureRequest {
    /// **ステップ**を並べる**タスク**の ID。[`DisclosureRow::Task`] が運んだ文字列そのもの。
    ///
    /// `null` は**現在地**の**タスク**である — 面を開いた時点の形がこれであり、
    /// **未着手**のときはどの**タスク**も開かない (spec Boundaries)。
    pub open_task_id: Option<String>,
}

/// **開示面**が描画に必要とするすべて (CAP-9 / FR-19)。
///
/// フィールド名は `src/overlay/Overlay.svelte` の `DisclosureSurface` 型と 1:1 で
/// 対応する。`invoke<T>` は実行時検査を行わないため、ここを変えると一覧が無言で
/// 空になる。契約は [`tests::the_disclosure_surface_keeps_its_wire_contract`] が固定する。
///
/// **表示のたびに取り直される。** 隠れている間の一覧を持ち越さない (AD-3 鮮度規則 /
/// FR-19「オーバーレイを閉じた時点で破棄」)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisclosureSurface {
    /// コアが `manage` されていないときの理由。読めていれば `None`。
    ///
    /// **面は開き、理由を示す** (I/O マトリクス「コア不在」)。空の一覧と区別が付かな
    /// ければ、状態を読めていないことが「タスクが 1 個も無い」として描かれる。
    pub state_error: Option<String>,
    /// 見出しと**ステップ**を一つの流れに並べた一覧。
    ///
    /// **タスク**が 1 個も無ければ空である。フロントはこれを見て「その旨の 1 行」を出す。
    pub rows: Vec<DisclosureRow>,
}

/// **コア状態を開示面の一覧へ落とす純粋関数** (CAP-9 / FR-19)。
///
/// `AppHandle` を取らない。コマンドに埋め込んだままでは、見出しと**ステップ**の並びを
/// 取り違えても生きた Tauri アプリを起動しない限り誰も気づかない ([`snapshot_of`] と
/// 同じ流儀)。
///
/// # v2 への申し送り
///
/// FR-19 は「**開示面**は**腐敗**した**タスク**を含まない」と定めるが、v1 に**腐敗**は
/// 存在しないため濾すものが無く、条件は自明に満たされる。**CAP-20 を足す時点で、除外を
/// 加える場所はここである。**
fn disclosure_of(state: Option<&CoreState>, open_task_id: Option<&str>) -> DisclosureSurface {
    let Some(state) = state else {
        return DisclosureSurface {
            state_error: Some(CORE_MISSING.to_string()),
            rows: Vec::new(),
        };
    };

    // **現在地**は一度だけ読む。行ごとに引き直すと、行の間で値が変わりうる形になる。
    let current = state.current_position().step_id();
    // **現在地**を抱える**タスク**も同じく一度だけ読む。閉じていても見出しがそれを示す。
    let here = task_of_current_position(state);
    // **開く**タスク**も一度だけ決める。** 高々一つしか真になりえないことを、この
    // 一つの値が構造として負う。
    let open = task_to_open(state, open_task_id);
    let mut rows = Vec::new();
    for task in state.tasks() {
        // **見出しは常にすべて並ぶ。** 隠れるのは**ステップ**だけである。
        let opened = open == Some(task.id());
        rows.push(DisclosureRow::Task {
            task_id: task.id().to_string(),
            title: task.title().to_string(),
            open: opened,
            holds_current_position: here == Some(task.id()),
        });
        if !opened {
            continue;
        }
        for step in task.steps() {
            rows.push(DisclosureRow::Step {
                step_id: step.id().to_string(),
                content: step.content().to_string(),
                completed: step.is_completed(),
                current: current == Some(step.id()),
                // **中断メモ**の本文はここに現れない。再開時の提示は CAP-8 の既定表示が
                // 担う (spec Never)。
            });
        }
    }

    DisclosureSurface {
        state_error: None,
        rows,
    }
}

/// **どの**タスク**の**ステップ**を並べるか決める純粋関数** (spec Boundaries)。
///
/// `requested` が `None` のときは**現在地**の**タスク**である — 面を開いた時点の形が
/// これであり、**未着手**には開く**タスク**が無い。
///
/// # 読めない ID・消えた ID はどの**タスク**も開かない
///
/// フロントが送るのは直前に描いた見出しの ID だけであり、一致しないのは一覧と実際の状態が
/// 食い違ったときに限られる。**そのとき別の**タスク**を勝手に開かない** — 確定した見出しと
/// 違うものが開けば、次の Enter が見ていない**ステップ**へ**現在地**を移す。理由を返さない
/// のは、開閉が表示の状態にすぎず、何も書かれていないためである。
fn task_to_open(state: &CoreState, requested: Option<&str>) -> Option<TaskId> {
    let Some(requested) = requested else {
        return task_of_current_position(state);
    };
    state.task(TaskId::parse(requested).ok()?).map(Task::id)
}

/// **現在地**を抱える**タスク** (CAP-6 / FR-6)。**未着手**なら `None`。
///
/// 面を開いた時点で開く**タスク**であり、閉じているときに見出しが示す**タスク**でもある。
/// **一つの読み方を二箇所に書かない** — 食い違えば、開いていない**タスク**に印が付く。
fn task_of_current_position(state: &CoreState) -> Option<TaskId> {
    let here = state.current_position().step_id()?;
    Some(state.task_of_step(here)?.id())
}

/// `switch_current_position` が受け取る要求。**これがコマンドの引数型そのものである。**
///
/// フロントは `invoke('switch_current_position', { request: { note, declareCompletion } })`
/// と呼ぶ。平らな引数にすると、コマンドの仮引数名が唯一の契約になり、Tauri を起動せずに
/// 検証できる型が一つも残らない — 仮引数を改名しても両方の検査が通ったまま、実行時に
/// 毎回の Enter が引数の復元で落ちる。型として持てば
/// `tests::the_request_keeps_its_wire_contract` が形を固定できる。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchRequest {
    /// 確定する**中断メモ**。`null` および空欄は**省略**であり、既存のメモを変えない。
    ///
    /// **提示された既存メモをそのまま送り返してはならない。** 送り返せば、読み返した
    /// だけの**切り替え**が「メモを書いた」として記録され、SM-C3 の記入率が膨らむ。
    /// 判定はフロント側 (`Overlay.svelte`) が持つ — 何が「今回書かれた」かを知って
    /// いるのは入力欄だけである。
    pub note: Option<String>,
    /// **完了**を宣言するか。宣言しない**切り替え**も同じく成立する (FR-4 / AD-2)。
    pub declare_completion: bool,
}

/// `create_task` が受け取る要求。**これがコマンドの引数型そのものである。**
///
/// フロントは
/// `invoke('create_task', { request: { title, steps, moveCurrentPosition } })` と呼ぶ。
/// [`SwitchRequest`] と同じ理由で名前付きの型として持つ — 平らな引数にすると、コマンドの
/// 仮引数名が唯一の契約になり、Tauri を起動せずに検証できる型が一つも残らない。
///
/// # なぜ**ステップ**を配列で受け取らないのか
///
/// 「1 行 = 1 **ステップ**」「前後の空白を除いて空になる行は落とす」という規則は
/// **一箇所にしか無いべきである**。配列で受け取れば、行を切り分けて落とす判断がフロントへ
/// 移り、[`as_task_definition`] の単体テストが守っているものが実際の経路から外れる。
/// 入力欄の文字列をそのまま渡し、整えるのはコマンド境界の純粋関数だけとする。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskRequest {
    /// **タスク**の題名。前後の空白を除いて空であってはならない (FR-4)。
    pub title: String,
    /// **ステップ**の入力欄の文字列そのもの。1 行 = 1 **ステップ**。
    pub steps: String,
    /// 作成に加えて**現在地**をその**タスク**の第 1 **ステップ**へ置くか。
    ///
    /// **書き留めることと着手することは別の行為である** (FR-18 と同じ原則)。既に
    /// **現在地**があるときも、置き換えはこれが真のときにだけ起きる。
    pub move_current_position: bool,
}

/// **タスク**の作成の結末。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskOutcome {
    /// **現在地**がその**タスク**の第 1 **ステップ**へ移ったか。
    ///
    /// **タスク**が生まれたことはこの欄に現れない — 作成が失敗したときにだけ `Err` が
    /// 返るため、`Ok` が返った時点で作成は確定している。**逆に、作成が確定した後は
    /// 決して `Err` を返さない**: 返せば「何も保存されていない」と示されたうえで同じ
    /// 入力が再確定され、v1 では削除も到達もできない重複した**タスク**が生まれる。
    /// 着手に失敗したことは `Err` ではなくこの欄の `false` として運ぶ。
    pub moved: bool,
}

/// `select_step` が受け取る要求。**これがコマンドの引数型そのものである。**
///
/// フロントは `invoke('select_step', { request: { stepId } })` と呼ぶ。[`SwitchRequest`]
/// と同じ理由で名前付きの型として持つ — 平らな引数にすると、コマンドの仮引数名が唯一の
/// 契約になり、Tauri を起動せずに検証できる型が一つも残らない。
///
/// # なぜ**中断メモ**の欄が無いのか
///
/// **この経路は中断メモの機会を与えない** (spec Design Notes)。欄が無ければ、一覧の中に
/// CAP-7 の儀式を作り直す経路が型として成立しない。**完了**の欄が無いのも同じである —
/// **完了**は**現在地**の移動で付与も取消もされない (FR-4 / AD-2)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectStepRequest {
    /// 選んだ**ステップ**の ID。[`DisclosureRow::Step`] が運んだ文字列そのもの。
    pub step_id: String,
}

/// **開示面**からの選択の結末。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectStepOutcome {
    /// **現在地**が動いたか。
    ///
    /// 既に**現在地**である**ステップ**を選んだときは偽であり、**何も書かれていない** —
    /// 履歴も増えない (I/O マトリクス「同じステップを選ぶ」)。
    pub moved: bool,
}

/// **イベントを発行すべきか決める純粋関数** ([`outcome_of`] と同じ流儀)。
///
/// 条件をコマンドに埋め込んだままでは、`if` を外して常時発行に変えても生きた Tauri
/// アプリを起動しない限り誰も気づかない。何も書かれていない選択で
/// `current_position_changed` を出せば、受け手は起きていない変化のために描き直す。
const fn announces_a_move(outcome: SelectStepOutcome) -> bool {
    outcome.moved
}

/// 選んだ行の ID が読めないことを示す理由。
///
/// 起こるのは一覧と実際の状態が食い違ったときだけである。黙って何もしないと、Enter が
/// 効かない理由が利用者に伝わらない。
const ROW_UNREADABLE: &str = "選んだ行を特定できない。一覧を開き直すこと。";

/// **選んだ行を**ステップ**の ID へ変える純粋関数** (I/O マトリクス「コア不在」の手前)。
///
/// `AppHandle` を取らない。**コアを要求する前にこれを通すことが、入力の不備に対して
/// 「状態を読み込めていない」という無関係な理由を返さないための順序である**
/// ([`as_task_definition`] と同じ流儀)。コマンドに埋め込んだままでは、読めない ID の
/// 扱いを生きた Tauri アプリ無しに一行も検証できない。
fn step_to_select(step_id: &str) -> Result<StepId, String> {
    StepId::parse(step_id).map_err(|_| ROW_UNREADABLE.to_string())
}

/// 入力から組み立てた、**タスク**の題名と**ステップ**の内容の列。
///
/// **`Task::create` は題名と内容の空白を素通しする** (`domain/task.rs`)。整えるのは
/// この型を作る側の責務であり、ドメインへ渡る時点では既に整っている。
#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskDefinition {
    title: String,
    step_contents: Vec<String>,
}

/// 題名が無いことを示す理由。**面を閉じずに提示される** (I/O マトリクス「題名が空」)。
const TITLE_MISSING: &str = "題名が空である。タスクには題名が要る。";

/// **ステップ**が 1 個も残らないことを示す理由 (I/O マトリクス「残る行が無い」)。
const STEPS_MISSING: &str = "ステップが 1 個も無い。ステップを持たないタスクは作れない。";

/// 1 行 = 1 **ステップ**を切り分ける区切り。
///
/// `\r\n` は空の断片を生むが、空行は落ちるため結果は変わらない。
const STEP_SEPARATORS: [char; 4] = ['\n', '\r', '\u{2028}', '\u{2029}'];

/// **入力を**タスク**の定義へ整える純粋関数** (I/O マトリクス「空行の混在」ほか)。
///
/// `AppHandle` を取らない。呼び出し側に埋め込んだままでは、生きた Tauri アプリを
/// 起動しない限り一行も検証できない ([`as_interruption_note`] と同じ流儀)。
///
/// # 規則
///
/// - 題名は前後の空白を除く。除いて空なら作らない (FR-4)。
/// - **ステップ**は 1 行 = 1 個。行の順がそのまま連番 1..N になる。
/// - 前後の空白を除いて空になる行は落とす。落とした結果 1 個も残らなければ作らない。
///
/// # 行の区切り
///
/// `str::lines` は `\n` (と直前の `\r`) しか行と認めない。入力欄から来る文字列は
/// 貼り付け元によって単独の `\r` や U+2028 / U+2029 を含みうるため、それらも区切りと
/// して扱う。扱わなければ複数行が 1 個の**ステップ**に潰れ、内容に制御文字が残る。
///
/// # なぜ**中断メモ**と扱いが違うのか
///
/// [`as_interruption_note`] は本文をそのまま渡す — 利用者が書いた形をこちらの都合で
/// 書き換えないためである。こちらは逆に整える。**行が区切りとして意味を持つ**入力では、
/// 行頭の空白や空行は「書いた形」ではなく入力の都合であり、そのまま**ステップ**の内容に
/// すると連番と内容の両方がずれる。
fn as_task_definition(title: &str, steps: &str) -> Result<TaskDefinition, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err(TITLE_MISSING.to_string());
    }

    let step_contents: Vec<String> = steps
        .split(STEP_SEPARATORS)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    if step_contents.is_empty() {
        return Err(STEPS_MISSING.to_string());
    }

    Ok(TaskDefinition {
        title: title.to_string(),
        step_contents,
    })
}

/// **着手すべき**ステップ**を選ぶ純粋関数** — 第 1 **ステップ**、すなわち連番が 1 の
/// もの (I/O マトリクス「作成して着手」)。
///
/// `AppHandle` を取らない。コマンドに埋め込んだままでは、並びの先頭と末尾を取り違えても
/// 生きた Tauri アプリを起動しない限り誰も気づかない。
///
/// # なぜ並びの先頭ではなく連番で選ぶのか
///
/// 「第 1 **ステップ**」は連番 1 の**ステップ**であって、たまたま先頭に積まれたものでは
/// ない。連番で選べば、並びと連番が食い違ったときに黙って別の**ステップ**へ着手する経路が
/// 存在しなくなる。
fn step_to_start_from(task: Option<&Task>) -> Option<StepId> {
    task?
        .steps()
        .iter()
        .find(|step| step.ordinal() == 1)
        .map(Step::id)
}

/// **切り替え**の結末。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchOutcome {
    /// **現在地**が次の**ステップ**へ移ったか。
    ///
    /// 最終**ステップ**からの**切り替え**では偽である。そのときもメモ・**完了**・
    /// 履歴は確定しており、**現在地**だけが動かない。
    pub moved: bool,
}

/// コアが返した移動先から結末を決める純粋関数。
const fn outcome_of(destination: Option<StepId>) -> SwitchOutcome {
    SwitchOutcome {
        moved: destination.is_some(),
    }
}

/// 表示のたびに呼ばれ、描画に必要な完全なスナップショットを返す。
///
/// **コアが読めなくても失敗しない。** 失敗させるとホットキーの登録結果まで道連れになり、
/// 唯一の呼び出し経路が死んでいる事実が伝わらなくなる。読めないことは
/// [`OverlaySnapshot::state_error`] として運ぶ。
///
/// # Errors
///
/// 起動処理が終わっていないとき。既定値で埋めない — 埋めればホットキーの登録失敗が
/// 「成功」として描画される。フロントは短い間隔で再試行する。
#[tauri::command]
pub fn get_overlay_snapshot<R: Runtime>(
    app: AppHandle<R>,
    status: State<'_, ResidentStatus>,
) -> Result<OverlaySnapshot, String> {
    let hotkey = status
        .hotkey()
        .ok_or_else(|| "常駐プロセスの起動処理がまだ完了していない".to_string())?;

    let state = app.try_state::<Core>().map(|core| core.snapshot());
    Ok(snapshot_of(hotkey, state.as_ref()))
}

/// **切り替え** — 離脱側の**中断メモ**の確定・**完了**の宣言 (任意)・**現在地**の移動・
/// **切り替え履歴**の追記を、コア側の単一のトランザクションで確定させる (CAP-7 / AD-5)。
///
/// # Errors
///
/// コアが `manage` されていないとき、**現在地**が**未着手**のとき、または永続化に
/// 失敗したとき。いずれの場合も状態は変わっていない。
#[tauri::command]
pub fn switch_current_position<R: Runtime>(
    app: AppHandle<R>,
    request: SwitchRequest,
) -> Result<SwitchOutcome, String> {
    let core = require_core(app.try_state::<Core>())?;

    let destination = core
        .switch_current_position(
            as_interruption_note(request.note),
            request.declare_completion,
        )
        .map_err(|error| error.to_string())?;
    let outcome = outcome_of(destination);

    // **動いたときだけ発行する。** 最終**ステップ**からの**切り替え**は**現在地**を
    // 動かさないため、そこで発行すれば起きていない変化を主張することになる。その経路で
    // 再描画が落ちることもない — 呼び出し側は戻り値を受け取った時点でスナップショットを
    // 取り直すためである (AD-3 鮮度規則)。
    if outcome.moved {
        crate::announce_current_position_changed(&app);
    }

    // 本文は書かない。書いてよいのは「起きた」という事実だけである。
    log::info!("a switch was committed (moved={})", outcome.moved);
    Ok(outcome)
}

/// **タスク**を作る (CAP-4 / FR-4)。
///
/// 題名と 1 行 = 1 **ステップ**の入力から**タスク**を生み、要求されていれば**現在地**を
/// その第 1 **ステップ**へ置く。
///
/// # なぜ作成と**現在地**の移動が一つのトランザクションではないのか
///
/// AD-5 が単一トランザクションを要求しているのは**切り替え**である — 離脱側のメモ・
/// **完了**・移動・履歴が割れると「メモは残ったが現在地が動いていない」という半端な状態が
/// 残るためである。こちらで割れて残るのは「**タスク**は生まれたが**現在地**が動いて
/// いない」であり、これは**作成のみの確定が正規に作る状態そのもの**である。半端な状態が
/// 存在しないため、コアへ新しい経路を足してまで束ねる理由が無い。
///
/// # 作成が確定した後は決して失敗しない
///
/// **ステップ**の引き当てと**現在地**の移動はどちらも失敗しうるが、その時点で**タスク**は
/// 既に永続化され `task_created` も発行されている。ここで `Err` を返すと、呼び出し側は
/// 「何も保存されていない」と示したうえで入力を保持し、利用者は自然に再確定する —
/// v1 には削除も、既存の**タスク**へ到達する経路 (CAP-9) も無いため、重複した**タスク**は
/// 二度と始末できない。着手できなかったことは [`CreateTaskOutcome::moved`] の `false`
/// として運び、記録はログに残す。
///
/// # Errors
///
/// 題名が空のとき、**ステップ**が 1 個も残らないとき、コアが `manage` されていないとき、
/// または**タスク**の永続化そのものに失敗したとき。**いずれの場合も何も保存されておらず、
/// 呼び出し側は面を閉じずに理由を提示して入力を保持する** (I/O マトリクス)。
#[tauri::command]
pub fn create_task<R: Runtime>(
    app: AppHandle<R>,
    request: CreateTaskRequest,
) -> Result<CreateTaskOutcome, String> {
    // **コアを要求する前に入力を検める。** 順を逆にすると、題名を書き忘れただけの利用者に
    // 「状態を読み込めていない」という無関係な理由が返りうる。
    let definition = as_task_definition(&request.title, &request.steps)?;
    let core = require_core(app.try_state::<Core>())?;

    let task_id = core
        .create_task(definition.title, definition.step_contents)
        .map_err(|error| error.to_string())?;
    // 状態を変えた後は必ず event を発行する (AD-3)。ペイロードは持たない — 受け手は
    // スナップショットを取り直す。
    crate::announce_task_created(&app);

    if !request.move_current_position {
        // 題名も**ステップ**の内容も書かない。書いてよいのは「起きた」という事実だけである。
        log::info!("a task was created (moved=false)");
        return Ok(CreateTaskOutcome { moved: false });
    }

    // ここから先の失敗は `Err` にしない。**タスク**は既に確定している。
    let state = core.snapshot();
    let Some(first) = step_to_start_from(state.task(task_id)) else {
        log::error!(
            "the created task had no step with ordinal 1; the current position was left as it was"
        );
        return Ok(CreateTaskOutcome { moved: false });
    };
    if let Err(error) = core.move_current_position(first) {
        log::error!("a task was created but the current position could not be moved: {error}");
        return Ok(CreateTaskOutcome { moved: false });
    }
    crate::announce_current_position_changed(&app);

    log::info!("a task was created (moved=true)");
    Ok(CreateTaskOutcome { moved: true })
}

/// **開示面**が表示されるたびに呼ばれ、一覧の完全なスナップショットを返す
/// (CAP-9 / FR-19 / AD-3 鮮度規則)。
///
/// 見出しを確定したときも同じ経路を通る。**開く**タスク**が変わっただけで一覧を作り直す**
/// — 描画側に濾させると、線に乗った時点で全体像が既に渡っていることになる。
///
/// **コアが読めなくても失敗しない。** 面は開き、理由を [`DisclosureSurface::state_error`]
/// として運ぶ (I/O マトリクス「コア不在」)。失敗させると、開いた面が空白のまま出る。
///
/// # なぜ [`get_overlay_snapshot`] に相乗りしないのか
///
/// 既定表示は**次の一手**のみを描く (FR-2)。一覧を同じスナップショットに載せれば、
/// 初期表示のために毎回取得される値の中に全体像が入り、**隠れている間持ち越さない**
/// という FR-19 の条件を保つ場所が無くなる。取得の契機が違うものは別のコマンドにする。
#[tauri::command]
pub fn get_disclosure_surface<R: Runtime>(
    app: AppHandle<R>,
    request: DisclosureRequest,
) -> DisclosureSurface {
    let state = app.try_state::<Core>().map(|core| core.snapshot());
    disclosure_of(state.as_ref(), request.open_task_id.as_deref())
}

/// **開示面**で選んだ**ステップ**へ**現在地**を移す (CAP-9 / FR-19)。
///
/// **現在地**の移動と**切り替え履歴**の追記は、コア側の単一のトランザクションで確定する
/// (AD-5)。**中断メモ**の機会は与えず、**完了**にも触れない — 履歴の「メモを書いたか」は
/// 常に偽である (spec Design Notes)。
///
/// # Errors
///
/// 選んだ行の ID が読めないとき、コアが `manage` されていないとき、**ステップ**が
/// 見つからないとき、または永続化に失敗したとき。**いずれの場合も状態は変わっていない。**
#[tauri::command]
pub fn select_step<R: Runtime>(
    app: AppHandle<R>,
    request: SelectStepRequest,
) -> Result<SelectStepOutcome, String> {
    // **コアを要求する前に入力を検める** (`create_task` と同じ順序)。逆にすると、行の
    // 取り違えに対して「状態を読み込めていない」という無関係な理由が返りうる。
    let step_id = step_to_select(&request.step_id)?;
    let core = require_core(app.try_state::<Core>())?;

    let outcome = SelectStepOutcome {
        moved: core
            .select_step(step_id)
            .map_err(|error| error.to_string())?,
    };

    // **動いたときだけ発行する。** 何も書かれていない選択でこれを出せば、起きていない
    // 変化を主張することになる。
    if announces_a_move(outcome) {
        crate::announce_current_position_changed(&app);
    }

    // **ステップ**の内容も**タスク**の題名も書かない。書いてよいのは「起きた」という
    // 事実だけである。
    log::info!(
        "a step was selected on the disclosure surface (moved={})",
        outcome.moved
    );
    Ok(outcome)
}

/// 入力欄の文字列を**中断メモ**に変える。**空欄は省略である** (FR-7)。
///
/// 空白のみの入力を「メモを書いた」として記録すると、SM-C3 の記入率が中身のない
/// 打鍵で膨らむ。逆に本文はそのまま渡す — 前後の空白を削ると、利用者が書いた形を
/// こちらの都合で書き換えることになる。
fn as_interruption_note(note: Option<String>) -> Option<InterruptionNote> {
    let text = note?;
    if text.trim().is_empty() {
        return None;
    }
    Some(InterruptionNote::new(text))
}

/// オーバーレイを閉じる (Esc・フォーカス離脱)。隠した上で直前に最前面だったアプリへ
/// フォーカスを返す。
///
/// 失敗を成功に潰さない。ウィンドウが取得できない場合は `Err` を返し、フロントは
/// 代替経路へ倒れる。
#[tauri::command]
pub fn hide_overlay<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    log::info!("the overlay asked to be closed");
    presentation::hide(&app)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// フロントが代替経路 (`getCurrentWindow().hide()`) で自ら隠したことを伝える。
///
/// `hide_overlay` が失敗した後の保険である。これを伝えないと、コア側の可視状態の記録が
/// 「表示中」のまま残り、次のホットキー押下が「隠す」に倒れて無反応になる。
#[tauri::command]
pub fn mark_overlay_hidden() {
    presentation::mark_hidden();
}

/// 起動時に確定したホットキーの登録結果を公開する。
pub fn publish_hotkey_status<R: Runtime>(app: &AppHandle<R>, status: HotkeyStatus) {
    app.state::<ResidentStatus>().set_hotkey(status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::FixedClock;
    use crate::ports::storage::{Commit, RestoredState, Storage, StorageError};

    /// 何でも受け付けるストレージ。**コア状態を組み立てるためだけに使う。**
    struct AcceptingStorage;

    impl Storage for AcceptingStorage {
        fn restore(&self) -> Result<RestoredState, StorageError> {
            Ok(RestoredState::default())
        }

        fn apply(&self, _commit: &Commit) -> Result<(), StorageError> {
            Ok(())
        }
    }

    fn contents(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    /// 第 3/全 6 を指すコア状態を作る。
    fn a_state_at_the_third_of_six() -> CoreState {
        let core = Core::restore(
            Box::new(FixedClock::at(1_789_000_000_000)),
            Box::new(AcceptingStorage),
        )
        .expect("空の状態は復元できる");
        let task_id = core
            .create_task(
                "原稿",
                contents(&["一", "二", "3 段落目を書き直す", "四", "五", "六"]),
            )
            .expect("作れる");
        let third = core.snapshot().task(task_id).expect("ある").steps()[2].id();
        core.move_current_position(third).expect("移せる");
        core.set_interruption_note(third, Some(InterruptionNote::new("接続詞を整える途中")))
            .expect("メモを置ける");
        core.snapshot()
    }

    /// スナップショットがまだ確定していない間は成功を返さない。
    ///
    /// 「未確定」を既定値で埋めて返すと、ホットキーの登録失敗が「成功」として
    /// 描画されうる。フロントが再試行できるよう、未確定は失敗として表現する。
    #[test]
    fn an_unresolved_status_is_not_reported_as_success() {
        let status = ResidentStatus::default();
        assert!(status.hotkey().is_none());
    }

    /// 確定後は同じ値が読み出せる。
    #[test]
    fn a_published_status_is_readable() {
        let status = ResidentStatus::default();
        status.set_hotkey(HotkeyStatus::failed("衝突".to_string()));

        let hotkey = status.hotkey().expect("確定後は値が読める");
        assert!(!hotkey.registered);
        assert!(hotkey.error.is_some());
    }

    /// I/O マトリクス「コア不在」— 状態を書き換える経路では明示的なエラーになる。
    ///
    /// 文面まで固定するのは、これが利用者に示される唯一の手がかりだからである
    /// (復元の失敗は他にログしか経路を持たない)。
    #[test]
    fn a_missing_core_becomes_an_explicit_error() {
        let outcome = require_core::<&Core>(None);
        assert_eq!(outcome.err().as_deref(), Some(CORE_MISSING));
        assert!(
            CORE_MISSING.contains("現在地"),
            "何が失われているかを利用者に示す文であること"
        );
    }

    /// `manage` されていれば、そのまま通す。不在の扱いが常時エラーになっていない。
    #[test]
    fn a_present_core_passes_through() {
        assert_eq!(require_core(Some("core")), Ok("core"));
    }

    /// I/O マトリクス「コア不在」— **表示する経路ではホットキーを道連れにしない。**
    ///
    /// DB が開けずホットキーの登録も失敗している状況で、コマンドごと失敗させると
    /// 「唯一の呼び出し経路が死んでいる」という事実が誰にも伝わらない
    /// (spec Never「`hotkey` を失わせない」)。
    #[test]
    fn a_missing_core_still_reports_the_hotkey() {
        let snapshot = snapshot_of(HotkeyStatus::failed("衝突".to_string()), None);

        assert!(!snapshot.hotkey.registered, "ホットキーの失敗が残っている");
        assert_eq!(snapshot.state_error.as_deref(), Some(CORE_MISSING));
        assert_eq!(snapshot.step_content, None);
    }

    /// I/O マトリクス「既定表示」— 第 3/全 6 が、位置情報とメモごと取り出される。
    ///
    /// **`stepOrdinal` と `stepCount` の取り違えをここで落とす。** 両方とも数であり、
    /// 入れ替わっても型では気づけない。
    #[test]
    fn the_third_of_six_projects_its_content_position_and_note() {
        let snapshot = snapshot_of(
            HotkeyStatus::registered(),
            Some(&a_state_at_the_third_of_six()),
        );

        assert_eq!(snapshot.state_error, None);
        assert_eq!(snapshot.step_content.as_deref(), Some("3 段落目を書き直す"));
        assert_eq!(snapshot.step_ordinal, Some(3), "第 N の N");
        assert_eq!(snapshot.step_count, Some(6), "全 M の M");
        assert_eq!(
            snapshot.interruption_note.as_deref(),
            Some("接続詞を整える途中")
        );
    }

    /// **内容が出るなら全体数も必ず出る。** 片方だけ空の組み合わせを作らない。
    #[test]
    fn a_shown_step_always_carries_its_count() {
        let snapshot = snapshot_of(
            HotkeyStatus::registered(),
            Some(&a_state_at_the_third_of_six()),
        );

        assert_eq!(
            snapshot.step_content.is_some(),
            snapshot.step_count.is_some(),
            "内容と全体数は同時に決まる"
        );
        assert_eq!(
            snapshot.step_content.is_some(),
            snapshot.step_ordinal.is_some()
        );
    }

    /// I/O マトリクス「未着手」— 空欄ではなく、未着手と分かる形で返る。
    #[test]
    fn a_not_started_position_projects_as_nothing_shown() {
        let snapshot = snapshot_of(HotkeyStatus::registered(), Some(&CoreState::default()));

        assert_eq!(snapshot.state_error, None, "読めてはいる");
        assert_eq!(snapshot.step_content, None);
        assert_eq!(snapshot.step_ordinal, None);
        assert_eq!(snapshot.step_count, None);
        assert_eq!(snapshot.interruption_note, None);
    }

    /// Rust → TS の契約。フィールド名を変えるとフロントの警告が無言で消えるため、
    /// 実際に送られる JSON の形をここで固定する。
    #[test]
    fn the_snapshot_keeps_its_wire_contract() {
        let snapshot = OverlaySnapshot {
            hotkey: HotkeyStatus::failed("衝突".to_string()),
            state_error: None,
            step_content: Some("下書きを 3 段落まで書く".to_string()),
            step_ordinal: Some(3),
            step_count: Some(6),
            interruption_note: Some("3 段落目の途中".to_string()),
        };
        let json: serde_json::Value =
            serde_json::to_value(&snapshot).expect("スナップショットは直列化できる");

        let hotkey = json
            .get("hotkey")
            .expect("`hotkey` は Overlay.svelte の OverlaySnapshot が読む名前");
        assert!(hotkey.get("accelerator").is_some_and(|v| v.is_string()));
        assert!(hotkey.get("registered").is_some_and(|v| v.is_boolean()));
        assert!(hotkey.get("error").is_some_and(|v| v.is_string()));

        // **既存の 3 コマンドと `hotkey` 欄を壊さないことが本スライスの制約である。**
        assert!(json.get("stepContent").is_some_and(|v| v.is_string()));
        assert!(json.get("stepOrdinal").is_some_and(|v| v.is_u64()));
        assert!(json.get("stepCount").is_some_and(|v| v.is_u64()));
        assert!(json.get("interruptionNote").is_some_and(|v| v.is_string()));

        let ok = serde_json::to_value(OverlaySnapshot {
            hotkey: HotkeyStatus::registered(),
            state_error: None,
            step_content: None,
            step_ordinal: None,
            step_count: None,
            interruption_note: None,
        })
        .expect("成功時も直列化できる");
        assert!(
            ok["hotkey"]["error"].is_null(),
            "成功時の error は null であり、フロントの `string | null` と一致する"
        );
        // **未着手**は空欄ではなく null として運ばれる。フロントはこれを見て
        // 「未着手である旨の 1 行」を出す。
        assert!(ok["stateError"].is_null());
        assert!(ok["stepContent"].is_null());
        assert!(ok["stepOrdinal"].is_null());
        assert!(ok["stepCount"].is_null());
        assert!(ok["interruptionNote"].is_null());
    }

    /// **切り替え履歴に由来する欄が境界に一つも無い** (AD-15)。
    ///
    /// 欄が無ければ、フロントがどう書こうと表示できる値が存在しない。
    #[test]
    fn the_snapshot_carries_nothing_from_the_switch_record() {
        let json: serde_json::Value = serde_json::to_value(OverlaySnapshot {
            hotkey: HotkeyStatus::registered(),
            state_error: None,
            step_content: Some("下書き".to_string()),
            step_ordinal: Some(1),
            step_count: Some(2),
            interruption_note: None,
        })
        .expect("直列化できる");

        // `serde_json` のオブジェクトは辞書順で並ぶ。順序ではなく**集合**を固定する。
        let fields: Vec<&String> = json
            .as_object()
            .expect("オブジェクトである")
            .keys()
            .collect();
        assert_eq!(
            fields,
            vec![
                "hotkey",
                "interruptionNote",
                "stateError",
                "stepContent",
                "stepCount",
                "stepOrdinal"
            ],
            "履歴・件数・記入率に由来する欄を足さない (AD-15)"
        );
    }

    /// **TS → Rust の契約。** フロントが送る JSON が、コマンドの引数型へそのまま復元
    /// できることを固定する。ここが合っていなければ、毎回の Enter が実行時に引数の
    /// 復元で落ちる — 両方の検査が緑のままで。
    #[test]
    fn the_request_keeps_its_wire_contract() {
        let written: SwitchRequest =
            serde_json::from_str(r#"{"note":"3 段落目の途中","declareCompletion":true}"#)
                .expect("フロントが送る形で復元できる");
        assert_eq!(
            written,
            SwitchRequest {
                note: Some("3 段落目の途中".to_string()),
                declare_completion: true,
            }
        );

        // メモの省略は `null` で運ばれる。
        let omitted: SwitchRequest =
            serde_json::from_str(r#"{"note":null,"declareCompletion":false}"#)
                .expect("省略した形でも復元できる");
        assert_eq!(
            omitted,
            SwitchRequest {
                note: None,
                declare_completion: false,
            }
        );

        // `declareCompletion` を snake_case で送っても復元できてはならない。
        assert!(
            serde_json::from_str::<SwitchRequest>(r#"{"note":null,"declare_completion":true}"#)
                .is_err(),
            "受け付ける綴りは camelCase の一つだけである"
        );
    }

    /// **切り替え**の結末も TS 側と 1:1 である。移動の有無が `moved` に写る。
    #[test]
    fn the_switch_outcome_keeps_its_wire_contract() {
        let moved = outcome_of(Some(StepId::new(
            crate::domain::Timestamp::from_unix_millis(0),
        )));
        assert!(moved.moved, "移動先があれば真");
        assert_eq!(
            serde_json::to_value(moved).expect("直列化できる"),
            serde_json::json!({ "moved": true })
        );

        // 最終ステップ。メモと完了は確定しているが現在地は動かない。
        let stayed = outcome_of(None);
        assert!(!stayed.moved);
        assert_eq!(
            serde_json::to_value(stayed).expect("直列化できる"),
            serde_json::json!({ "moved": false })
        );
    }

    /// 空欄は**省略**である。既存のメモを消さないための唯一の判定点 (FR-7)。
    #[test]
    fn an_empty_input_is_an_omission_not_an_empty_note() {
        assert_eq!(as_interruption_note(None), None);
        assert_eq!(as_interruption_note(Some(String::new())), None);
        assert_eq!(as_interruption_note(Some("   \n\t ".to_string())), None);
    }

    // --- タスクの作成 (CAP-4 / FR-4) -----------------------------------------

    /// I/O マトリクス「作成のみ」— 題名と 3 行が、そのまま 1..3 の内容の列になる。
    #[test]
    fn a_title_and_three_lines_become_three_steps_in_order() {
        let definition = as_task_definition("原稿", "構成を決める\n下書きを書く\n推敲する")
            .expect("題名と行があれば作れる");

        assert_eq!(definition.title, "原稿");
        assert_eq!(
            definition.step_contents,
            contents(&["構成を決める", "下書きを書く", "推敲する"]),
            "行の順がそのまま連番 1..N になる"
        );
    }

    /// I/O マトリクス「空行の混在」— 行間と末尾の空行は落ち、残りの順序が保たれる。
    #[test]
    fn blank_lines_are_dropped_and_the_rest_keeps_its_order() {
        let definition = as_task_definition(
            "原稿",
            "構成を決める\n\n   \n下書きを書く\n\t\n推敲する\n\n   \n",
        )
        .expect("空行を落としても残る");

        assert_eq!(
            definition.step_contents,
            contents(&["構成を決める", "下書きを書く", "推敲する"]),
            "落ちるのは空行だけであり、残りの順序は動かない"
        );
    }

    /// 行ごとの前後の空白は除く。**行が区切りとして意味を持つ入力である。**
    ///
    /// 残したまま**ステップ**の内容にすると、字下げが内容の一部として保存される。
    #[test]
    fn each_line_is_trimmed() {
        let definition =
            as_task_definition("  原稿  ", "  構成を決める  \n\t下書きを書く\t").expect("作れる");

        assert_eq!(definition.title, "原稿");
        assert_eq!(
            definition.step_contents,
            contents(&["構成を決める", "下書きを書く"])
        );
    }

    /// I/O マトリクス「題名が空」— 作らない。理由は利用者に示される文である。
    #[test]
    fn a_blank_title_is_refused_with_a_reason() {
        assert_eq!(
            as_task_definition("   \t ", "構成を決める")
                .err()
                .as_deref(),
            Some(TITLE_MISSING)
        );
        assert_eq!(
            as_task_definition("", "構成を決める").err().as_deref(),
            Some(TITLE_MISSING)
        );
    }

    /// I/O マトリクス「残る行が無い」— **ステップ**を持たない**タスク**は作れない (FR-4)。
    ///
    /// 空白のみの行は落ちるため、**落とした結果 0 個**という経路も同じ理由になる。
    #[test]
    fn a_task_without_any_step_is_refused_with_a_reason() {
        assert_eq!(
            as_task_definition("原稿", "").err().as_deref(),
            Some(STEPS_MISSING)
        );
        assert_eq!(
            as_task_definition("原稿", "\n   \n\t\n").err().as_deref(),
            Some(STEPS_MISSING)
        );
    }

    /// 改行の綴りが違っても行は行である。入力欄から来る文字列は貼り付け元で揺れうる。
    ///
    /// 単独の `\r` と U+2028 / U+2029 を区切りとして扱わないと、複数行が 1 個の
    /// **ステップ**に潰れ、内容に制御文字が残ったまま保存される。
    #[test]
    fn every_line_separator_splits_steps() {
        for separator in ["\r\n", "\r", "\n", "\u{2028}", "\u{2029}"] {
            let steps = format!("構成を決める{separator}下書きを書く{separator}");
            let definition = as_task_definition("原稿", &steps)
                .unwrap_or_else(|_| panic!("{separator:?} は区切りである"));

            assert_eq!(
                definition.step_contents,
                contents(&["構成を決める", "下書きを書く"]),
                "区切り {separator:?} で 2 個に分かれる"
            );
        }
    }

    /// 区切りが混ざっていても、制御文字が内容に残らない。
    #[test]
    fn no_step_carries_a_line_separator() {
        let definition =
            as_task_definition("原稿", "一\r\n二\r三\u{2028}四\u{2029}五").expect("作れる");

        assert_eq!(
            definition.step_contents,
            contents(&["一", "二", "三", "四", "五"])
        );
        for content in &definition.step_contents {
            assert!(
                !content.contains(STEP_SEPARATORS),
                "内容に区切りが残っていない: {content:?}"
            );
        }
    }

    /// **着手先は連番 1 の**ステップ**である** (I/O マトリクス「作成して着手」)。
    ///
    /// 並びの先頭で選ぶ実装と結果が一致する状況でも、選んでいるのが連番であることを
    /// 固定する — 末尾を選ぶ実装に変えたなら、ここが落ちる。
    #[test]
    fn the_step_to_start_from_is_the_one_numbered_one() {
        let core = Core::restore(
            Box::new(FixedClock::at(1_789_000_000_000)),
            Box::new(AcceptingStorage),
        )
        .expect("空の状態は復元できる");
        let task_id = core
            .create_task(
                "原稿",
                contents(&["構成を決める", "下書きを書く", "推敲する"]),
            )
            .expect("作れる");

        let state = core.snapshot();
        let chosen = step_to_start_from(state.task(task_id)).expect("第 1 ステップがある");
        let task = state.task(task_id).expect("ある");
        assert_eq!(
            task.step(chosen).expect("ある").ordinal(),
            1,
            "連番 1 である"
        );
        assert_eq!(task.step(chosen).expect("ある").content(), "構成を決める");
        assert_ne!(
            chosen,
            task.steps().last().expect("ある").id(),
            "末尾ではない"
        );
    }

    /// 選んだ**ステップ**へ移した後、**現在地**は確かにそれを指す。
    ///
    /// **`move_current_position` の呼び出しを消しても `moved: true` を返せてしまう**
    /// 経路をここで塞ぐ。
    #[test]
    fn starting_a_created_task_points_the_current_position_at_its_first_step() {
        let core = Core::restore(
            Box::new(FixedClock::at(1_789_000_000_000)),
            Box::new(AcceptingStorage),
        )
        .expect("空の状態は復元できる");
        let task_id = core
            .create_task("原稿", contents(&["構成を決める", "下書きを書く"]))
            .expect("作れる");
        assert!(
            core.current_position().step_id().is_none(),
            "作成そのものは現在地に触れない"
        );

        let chosen = step_to_start_from(core.snapshot().task(task_id)).expect("ある");
        core.move_current_position(chosen).expect("移せる");

        assert_eq!(core.current_position().step_id(), Some(chosen));
        assert_eq!(core.current_position().task_id(), Some(task_id));
    }

    /// **タスク**が引けなければ着手先も無い。ここが `None` を返すことが、コマンドが
    /// `Err` ではなく `moved: false` を返す経路の入口である。
    #[test]
    fn a_missing_task_has_no_step_to_start_from() {
        assert_eq!(step_to_start_from(None), None);
    }

    /// **TS → Rust の契約。** フロントが送る JSON がそのまま復元できる。
    #[test]
    fn the_creation_request_keeps_its_wire_contract() {
        let request: CreateTaskRequest = serde_json::from_str(
            r#"{"title":"原稿","steps":"構成を決める\n下書きを書く","moveCurrentPosition":true}"#,
        )
        .expect("フロントが送る形で復元できる");
        assert_eq!(
            request,
            CreateTaskRequest {
                title: "原稿".to_string(),
                steps: "構成を決める\n下書きを書く".to_string(),
                move_current_position: true,
            }
        );

        // 作成のみの確定。**現在地**は動かない。
        let only: CreateTaskRequest = serde_json::from_str(
            r#"{"title":"原稿","steps":"構成を決める","moveCurrentPosition":false}"#,
        )
        .expect("作成のみの形でも復元できる");
        assert!(!only.move_current_position);

        // 受け付ける綴りは camelCase の一つだけである。
        assert!(serde_json::from_str::<CreateTaskRequest>(
            r#"{"title":"原稿","steps":"構成を決める","move_current_position":true}"#
        )
        .is_err());
    }

    /// 作成の結末も TS 側と 1:1 である。
    ///
    /// **欄は `moved` の一つだけである。** 件数・登録数に由来する欄を足さない —
    /// SM-C1 は登録数の増加を目標にしてはならないと定めており、境界に欄が無ければ
    /// フロントがどう書こうと描ける値が存在しない (AD-15)。
    #[test]
    fn the_creation_outcome_keeps_its_wire_contract() {
        let json = serde_json::to_value(CreateTaskOutcome { moved: true }).expect("直列化できる");
        assert_eq!(json, serde_json::json!({ "moved": true }));

        let stayed =
            serde_json::to_value(CreateTaskOutcome { moved: false }).expect("直列化できる");
        assert_eq!(stayed, serde_json::json!({ "moved": false }));
    }

    /// 整えた定義はドメインがそのまま受け取れる。**空白は既にここで落ちている。**
    ///
    /// `Task::create` は題名と内容の空白を素通しする (`domain/task.rs`)。整える責務が
    /// こちらにあることを、実際に**タスク**を作って確かめる。
    #[test]
    fn a_definition_reaches_the_domain_already_tidied() {
        let core = Core::restore(
            Box::new(FixedClock::at(1_789_000_000_000)),
            Box::new(AcceptingStorage),
        )
        .expect("空の状態は復元できる");

        let definition = as_task_definition("  原稿 ", " 構成を決める \n\n 下書きを書く \n")
            .expect("整えられる");
        let task_id = core
            .create_task(definition.title, definition.step_contents)
            .expect("作れる");

        let state = core.snapshot();
        let task = state.task(task_id).expect("ある");
        assert_eq!(task.title(), "原稿");
        assert_eq!(task.steps().len(), 2);
        assert_eq!(task.steps()[0].content(), "構成を決める");
        assert_eq!(task.steps()[0].ordinal(), 1, "連番は 1 から");
        assert_eq!(task.steps()[1].content(), "下書きを書く");
        assert_eq!(task.steps()[1].ordinal(), 2);
        // **作成は現在地に触れない** (`domain/state.rs`)。着手は別の打鍵に属する。
        assert!(core.current_position().step_id().is_none());
    }

    /// 本文はそのまま渡す。前後の空白も利用者が書いた形である。
    #[test]
    fn a_written_note_is_passed_through_unchanged() {
        assert_eq!(
            as_interruption_note(Some(" 3 段落目の途中 ".to_string()))
                .as_ref()
                .map(InterruptionNote::text),
            Some(" 3 段落目の途中 ")
        );
    }

    // --- 開示面 (CAP-9 / FR-19) ------------------------------------------------

    /// 2 **タスク**・計 5 **ステップ**のコア状態を作る。第 2 **タスク**の第 3 **ステップ**
    /// が**現在地**であり、第 1 **タスク**の第 2 **ステップ**は**完了**している。
    fn a_state_with_two_tasks_and_five_steps() -> CoreState {
        let core = Core::restore(
            Box::new(FixedClock::at(1_789_000_000_000)),
            Box::new(AcceptingStorage),
        )
        .expect("空の状態は復元できる");
        let first = core
            .create_task("原稿", contents(&["構成を決める", "下書きを書く"]))
            .expect("作れる");
        let second = core
            .create_task("買い物", contents(&["米", "味噌", "醤油"]))
            .expect("作れる");

        let snapshot = core.snapshot();
        let done = snapshot.task(first).expect("ある").steps()[1].id();
        let here = snapshot.task(second).expect("ある").steps()[2].id();
        core.declare_completion(done).expect("宣言できる");
        core.move_current_position(here).expect("移せる");
        core.snapshot()
    }

    /// **コア状態から直に ID を読む。** 射影の戻り値から取り出すと、全行が同じ ID を
    /// 運んでいても期待値と一致してしまう — そのとき一覧のどの行を選んでも同じ
    /// **ステップ**へ移る。
    fn step_id_in(state: &CoreState, task: usize, step: usize) -> String {
        state.tasks()[task].steps()[step].id().to_string()
    }

    /// 見出しの ID も同じ理由でコア状態から直に読む。**全見出しが同じ ID を運んでいれば、
    /// どの見出しを確定しても同じ**タスク**が開く。**
    fn task_id_in(state: &CoreState, task: usize) -> String {
        state.tasks()[task].id().to_string()
    }

    /// 開いている見出しの題名。**高々一つであることを併せて確かめる** (spec Boundaries)。
    fn opened_heading(surface: &DisclosureSurface) -> Option<String> {
        let mut open = surface.rows.iter().filter_map(|row| match row {
            DisclosureRow::Task {
                title, open: true, ..
            } => Some(title.clone()),
            _ => None,
        });
        let first = open.next();
        assert_eq!(open.next(), None, "開いている見出しは高々一つである");
        first
    }

    /// **現在地**を抱えると示している見出しの題名。**高々一つであることを併せて
    /// 確かめる** (CAP-6 / FR-6)。
    fn heading_with_the_current_position(surface: &DisclosureSurface) -> Option<String> {
        let mut marked = surface.rows.iter().filter_map(|row| match row {
            DisclosureRow::Task {
                title,
                holds_current_position: true,
                ..
            } => Some(title.clone()),
            _ => None,
        });
        let first = marked.next();
        assert_eq!(marked.next(), None, "現在地を抱える見出しは高々一つである");
        first
    }

    /// 一覧に現れる**ステップ**の内容。
    fn listed_steps(surface: &DisclosureSurface) -> Vec<String> {
        surface
            .rows
            .iter()
            .filter_map(|row| match row {
                DisclosureRow::Step { content, .. } => Some(content.clone()),
                DisclosureRow::Task { .. } => None,
            })
            .collect()
    }

    /// 受け入れ条件「面へ入る → 見出しは 2 件とも現れるが、**ステップ**は**現在地**の
    /// **タスク**の分だけが現れる」。
    ///
    /// **見出しと**ステップ**が一つの流れになっていること**を、並びそのもので見る。
    /// 開くのは**現在地**の**タスク**であり、もう一方の**ステップ**は一行も現れない。
    #[test]
    fn only_the_task_at_the_current_position_opens_its_steps() {
        let state = a_state_with_two_tasks_and_five_steps();

        let surface = disclosure_of(Some(&state), None);

        assert_eq!(surface.state_error, None);
        assert_eq!(
            surface.rows,
            vec![
                DisclosureRow::Task {
                    task_id: task_id_in(&state, 0),
                    title: "原稿".to_string(),
                    open: false,
                    holds_current_position: false,
                },
                DisclosureRow::Task {
                    task_id: task_id_in(&state, 1),
                    title: "買い物".to_string(),
                    open: true,
                    holds_current_position: true,
                },
                DisclosureRow::Step {
                    step_id: step_id_in(&state, 1, 0),
                    content: "米".to_string(),
                    completed: false,
                    current: false,
                },
                DisclosureRow::Step {
                    step_id: step_id_in(&state, 1, 1),
                    content: "味噌".to_string(),
                    completed: false,
                    current: false,
                },
                DisclosureRow::Step {
                    step_id: step_id_in(&state, 1, 2),
                    content: "醤油".to_string(),
                    completed: false,
                    current: true,
                },
            ],
            "見出しは 2 件とも現れ、ステップは開いた 1 タスクの分だけが続く"
        );
    }

    /// 見出しを確定した**タスク**が開き、**直前に開いていた**タスク**は閉じる**
    /// (spec Boundaries / I/O マトリクス「見出しを確定する」)。
    ///
    /// **開いた状態を積み上げられないことが、一覧が全体像へ戻らないための条件である。**
    /// 開けるのが一つだけであることを、内容そのもので見る — 5 件が同時に並んだ時点で
    /// 設計上の賭け #1 が破れている。
    #[test]
    fn opening_a_heading_closes_the_one_that_was_open() {
        let state = a_state_with_two_tasks_and_five_steps();

        let surface = disclosure_of(Some(&state), Some(&task_id_in(&state, 0)));

        assert_eq!(opened_heading(&surface).as_deref(), Some("原稿"));
        assert_eq!(
            listed_steps(&surface),
            vec!["構成を決める".to_string(), "下書きを書く".to_string()],
            "開いたタスクのステップだけが並ぶ"
        );
        assert_eq!(
            surface
                .rows
                .iter()
                .filter(|row| matches!(row, DisclosureRow::Task { .. }))
                .count(),
            2,
            "見出しは閉じたタスクの分も含めてすべて現れる"
        );
    }

    /// **見出しの確定は**現在地**を動かさない** (spec Boundaries)。
    ///
    /// 射影は状態を変えない純粋関数であり、開く**タスク**を変えても**現在地**の印は
    /// 開いた**タスク**の中にしか現れない。閉じた**タスク**にある**現在地**は、行ごと
    /// 一覧から消えるだけである。
    #[test]
    fn opening_a_heading_leaves_the_current_position_where_it_was() {
        let state = a_state_with_two_tasks_and_five_steps();

        let elsewhere = disclosure_of(Some(&state), Some(&task_id_in(&state, 0)));

        assert_eq!(
            elsewhere
                .rows
                .iter()
                .filter(|row| matches!(row, DisclosureRow::Step { current: true, .. }))
                .count(),
            0,
            "現在地のステップは閉じたタスクの中にあり、行が存在しない"
        );
        // **同じ状態から作り直せば、現在地は元の場所にある。** 開閉は表示の状態に
        // すぎず、コアに何も書いていない。
        let here = disclosure_of(Some(&state), Some(&task_id_in(&state, 1)));
        let DisclosureRow::Step { step_id, .. } = here
            .rows
            .iter()
            .find(|row| matches!(row, DisclosureRow::Step { current: true, .. }))
            .expect("現在地の行がある")
        else {
            unreachable!()
        };
        assert_eq!(*step_id, step_id_in(&state, 1, 2));
    }

    /// 読めない ID・どの**タスク**にも一致しない ID は、どれも開かない。
    ///
    /// **勝手に別の**タスク**を開かない。** 確定した見出しと違うものが開けば、次の Enter が
    /// 見ていない**ステップ**へ**現在地**を移す。
    #[test]
    fn an_unknown_open_task_leaves_every_task_closed() {
        let state = a_state_with_two_tasks_and_five_steps();

        for requested in ["not-a-uuid", "", "0198f0e0-0000-7000-8000-00000000ffff"] {
            let surface = disclosure_of(Some(&state), Some(requested));

            assert_eq!(opened_heading(&surface), None, "要求: {requested}");
            assert!(listed_steps(&surface).is_empty(), "要求: {requested}");
            assert_eq!(
                surface.rows.len(),
                2,
                "見出しは 2 件とも残る (要求: {requested})"
            );
        }
    }

    /// 行ごとの ID は別物である。**同じ ID を配ってしまえば、どの行を選んでも同じ
    /// ステップへ移る。**
    #[test]
    fn every_listed_step_carries_its_own_id() {
        let state = a_state_with_two_tasks_and_five_steps();
        let surface = disclosure_of(Some(&state), Some(&task_id_in(&state, 1)));

        let ids: Vec<String> = surface
            .rows
            .iter()
            .filter_map(|row| match row {
                DisclosureRow::Step { step_id, .. } => Some(step_id.clone()),
                DisclosureRow::Task { .. } => None,
            })
            .collect();
        assert_eq!(
            ids,
            vec![
                step_id_in(&state, 1, 0),
                step_id_in(&state, 1, 1),
                step_id_in(&state, 1, 2),
            ],
            "3 行が、コアが持つ 3 つのステップをそれぞれ指す"
        );
    }

    /// 見出しごとの ID も別物である。**同じ ID を配ってしまえば、どの見出しを確定しても
    /// 同じ**タスク**が開く。**
    #[test]
    fn every_heading_carries_its_own_id() {
        let state = a_state_with_two_tasks_and_five_steps();
        let surface = disclosure_of(Some(&state), None);

        let ids: Vec<String> = surface
            .rows
            .iter()
            .filter_map(|row| match row {
                DisclosureRow::Task { task_id, .. } => Some(task_id.clone()),
                DisclosureRow::Step { .. } => None,
            })
            .collect();
        assert_eq!(ids, vec![task_id_in(&state, 0), task_id_in(&state, 1)]);
    }

    /// **現在地**の行はちょうど一つである (CAP-6 / FR-6)。
    ///
    /// 集合として持たない**現在地**が、射影で二つに増えないことを見る。
    #[test]
    fn exactly_one_row_is_marked_as_the_current_position() {
        let surface = disclosure_of(Some(&a_state_with_two_tasks_and_five_steps()), None);

        let marked = surface
            .rows
            .iter()
            .filter(|row| matches!(row, DisclosureRow::Step { current: true, .. }))
            .count();
        assert_eq!(marked, 1);
    }

    /// **現在地**は、その**タスク**が閉じていても分かる (spec Boundaries)。
    ///
    /// **この検査が無いと、別の**タスク**を開いた瞬間に**現在地**がどこにも現れなくなる**
    /// — 凍結節が無条件に求めている「現在地が指す行がどれか分かること」が、一覧の形を
    /// 変えたことで静かに破れる。
    #[test]
    fn a_closed_task_still_shows_that_it_holds_the_current_position() {
        let state = a_state_with_two_tasks_and_five_steps();

        // **現在地**は第 2 タスクにあり、開いているのは第 1 タスクである。
        let surface = disclosure_of(Some(&state), Some(&task_id_in(&state, 0)));

        assert_eq!(opened_heading(&surface).as_deref(), Some("原稿"));
        assert!(
            !surface
                .rows
                .iter()
                .any(|row| matches!(row, DisclosureRow::Step { current: true, .. })),
            "現在地のステップは閉じたタスクの中にあり、行が存在しない"
        );
        assert_eq!(
            heading_with_the_current_position(&surface).as_deref(),
            Some("買い物"),
            "閉じていても、現在地を抱える見出しがそれと分かる"
        );
    }

    /// 開いている**タスク**が**現在地**を抱えているときも、印は見出しに付く。
    ///
    /// **開閉と取り違えていないことを見る。** `open` をそのまま返す実装にすると、開いた
    /// だけの**タスク**に現在地の印が付き、閉じた**現在地**からは消える。
    #[test]
    fn the_marker_follows_the_current_position_and_not_the_open_task() {
        let state = a_state_with_two_tasks_and_five_steps();

        let here = disclosure_of(Some(&state), None);
        assert_eq!(opened_heading(&here).as_deref(), Some("買い物"));
        assert_eq!(
            heading_with_the_current_position(&here).as_deref(),
            Some("買い物")
        );

        let elsewhere = disclosure_of(Some(&state), Some(&task_id_in(&state, 0)));
        assert_eq!(opened_heading(&elsewhere).as_deref(), Some("原稿"));
        assert_eq!(
            heading_with_the_current_position(&elsewhere).as_deref(),
            Some("買い物"),
            "開くタスクを変えても、印は現在地のあるタスクに留まる"
        );
    }

    /// **未着手**では、どの見出しにも印が付かない。
    ///
    /// **無い現在地を描かない。** 付ければ、まだ始めていないことが「ここにいる」として
    /// 静かに描かれる。
    #[test]
    fn a_not_started_position_marks_no_heading() {
        let core = Core::restore(
            Box::new(FixedClock::at(1_789_000_000_000)),
            Box::new(AcceptingStorage),
        )
        .expect("空の状態は復元できる");
        core.create_task("着手せずに書き留めた", contents(&["一", "二"]))
            .expect("作れる");
        let task = core
            .create_task("こちらも書き留めただけ", contents(&["三"]))
            .expect("作れる");

        for requested in [None, Some(task.to_string())] {
            let surface = disclosure_of(Some(&core.snapshot()), requested.as_deref());

            assert_eq!(
                heading_with_the_current_position(&surface),
                None,
                "未着手にはどの見出しにも印が付かない"
            );
        }
    }

    /// **未着手**でも一覧は出る。**現在地**の行が無いだけである。
    ///
    /// ここが空の一覧になると、着手せずに作った**タスク**へ到達する経路が消える —
    /// 本スライスが解消しようとしている穴そのものが残る。
    #[test]
    fn a_not_started_position_still_lists_every_task() {
        let core = Core::restore(
            Box::new(FixedClock::at(1_789_000_000_000)),
            Box::new(AcceptingStorage),
        )
        .expect("空の状態は復元できる");
        core.create_task("着手せずに書き留めた", contents(&["一", "二"]))
            .expect("作れる");

        let surface = disclosure_of(Some(&core.snapshot()), None);

        assert_eq!(surface.rows.len(), 1, "見出し 1 行だけである");
        assert_eq!(
            opened_heading(&surface),
            None,
            "未着手のときはどのタスクも開いていない (I/O マトリクス「未着手で開く」)"
        );
        assert!(
            !surface
                .rows
                .iter()
                .any(|row| matches!(row, DisclosureRow::Step { current: true, .. })),
            "現在地の行は無い"
        );
    }

    /// **未着手**でも見出しを確定すればその**タスク**が開く。
    ///
    /// ここが開かないと、着手せずに作った**タスク**へ到達する経路が消える — 本スライスが
    /// 解消しようとしている穴そのものが残る。
    #[test]
    fn a_not_started_position_can_still_open_a_heading() {
        let core = Core::restore(
            Box::new(FixedClock::at(1_789_000_000_000)),
            Box::new(AcceptingStorage),
        )
        .expect("空の状態は復元できる");
        let task = core
            .create_task("着手せずに書き留めた", contents(&["一", "二"]))
            .expect("作れる");

        let surface = disclosure_of(Some(&core.snapshot()), Some(&task.to_string()));

        assert_eq!(
            opened_heading(&surface).as_deref(),
            Some("着手せずに書き留めた")
        );
        assert_eq!(
            listed_steps(&surface),
            vec!["一".to_string(), "二".to_string()]
        );
    }

    /// **`None` は**現在地**の**タスク**、それ以外はその**タスク**である** (純粋関数)。
    ///
    /// 開く**タスク**の決め方をコマンドに埋め込んだままでは、`None` の意味を取り違えても
    /// 生きた Tauri アプリを起動しない限り誰も気づかない。
    #[test]
    fn the_open_task_defaults_to_the_one_holding_the_current_position() {
        let state = a_state_with_two_tasks_and_five_steps();

        assert_eq!(
            task_to_open(&state, None),
            Some(state.tasks()[1].id()),
            "面を開いた時点で開くのは現在地のタスクである"
        );
        assert_eq!(
            task_to_open(&state, Some(&task_id_in(&state, 0))),
            Some(state.tasks()[0].id()),
            "指定があればそのタスクである"
        );
        assert_eq!(task_to_open(&state, Some("not-a-uuid")), None);
        assert_eq!(
            task_to_open(&CoreState::default(), None),
            None,
            "未着手にはどのタスクも開かない"
        );
    }

    /// I/O マトリクス「タスクが無い」— 空の一覧であり、理由は付かない。
    ///
    /// **「読めていない」と「1 個も無い」を区別する。** フロントはこの違いで出す 1 行を
    /// 決める。
    #[test]
    fn an_empty_store_projects_as_an_empty_list_without_a_reason() {
        let surface = disclosure_of(Some(&CoreState::default()), None);

        assert_eq!(surface.state_error, None, "読めてはいる");
        assert!(surface.rows.is_empty());
    }

    /// I/O マトリクス「コア不在」— 面は開き、理由を運ぶ。一覧は空である。
    #[test]
    fn a_missing_core_opens_the_surface_with_a_reason() {
        let surface = disclosure_of(None, None);

        assert_eq!(surface.state_error.as_deref(), Some(CORE_MISSING));
        assert!(surface.rows.is_empty(), "描く一覧は無い");
    }

    /// **中断メモ**の本文が一覧に乗らない (spec Never)。
    ///
    /// 再開時の提示は CAP-8 の既定表示が担う。ここに載せれば、全体像を見るだけの操作で
    /// 他の**ステップ**の文脈まで視界に入る。
    #[test]
    fn the_disclosure_carries_no_interruption_note_text() {
        let core = Core::restore(
            Box::new(FixedClock::at(1_789_000_000_000)),
            Box::new(AcceptingStorage),
        )
        .expect("空の状態は復元できる");
        let task_id = core
            .create_task("原稿", contents(&["構成を決める", "下書きを書く"]))
            .expect("作れる");
        let first = core.snapshot().task(task_id).expect("ある").steps()[0].id();
        core.set_interruption_note(first, Some(InterruptionNote::new("接続詞を整える途中")))
            .expect("メモを置ける");

        // **タスク**を開いて**ステップ**の行を実際に並べる。閉じたままでは行が一つも
        // 無く、本文が現れないことが何も意味しない。
        let surface = disclosure_of(Some(&core.snapshot()), Some(&task_id.to_string()));
        assert_eq!(listed_steps(&surface).len(), 2, "ステップの行が並んでいる");
        let json = serde_json::to_string(&surface).expect("直列化できる");

        assert!(
            !json.contains("接続詞を整える途中"),
            "本文がどの欄にも現れない: {json}"
        );
        assert!(
            !json.contains("interruptionNote"),
            "欄そのものが無い: {json}"
        );
    }

    /// Rust → TS の契約。フィールド名を変えると一覧が無言で空になる。
    #[test]
    fn the_disclosure_surface_keeps_its_wire_contract() {
        let json: serde_json::Value = serde_json::to_value(DisclosureSurface {
            state_error: None,
            rows: Vec::new(),
        })
        .expect("直列化できる");

        let fields: Vec<&String> = json
            .as_object()
            .expect("オブジェクトである")
            .keys()
            .collect();
        assert_eq!(
            fields,
            vec!["rows", "stateError"],
            "件数・進捗率に由来する欄を足さない (AD-15)"
        );
        assert!(json["stateError"].is_null());
        assert!(json["rows"].is_array());
    }

    /// **鍵の集合を固定する検査** (`the_snapshot_carries_nothing_from_the_switch_record`
    /// と同じ流儀)。
    ///
    /// 欄が無ければ、フロントがどう書こうと描ける値が存在しない。`ordinal` を足せば
    /// 「第 N / 全 M」をこの面で組み立てられてしまい、`completedAt` を足せば時刻が
    /// 出る。どちらも AD-15 と spec Never が禁じている。
    #[test]
    fn the_disclosure_rows_keep_their_wire_contract() {
        let heading = serde_json::to_value(DisclosureRow::Task {
            task_id: "0198f0e0-0000-7000-8000-000000000000".to_string(),
            title: "原稿".to_string(),
            open: false,
            holds_current_position: true,
        })
        .expect("直列化できる");
        assert_eq!(
            heading
                .as_object()
                .expect("オブジェクトである")
                .keys()
                .collect::<Vec<&String>>(),
            vec!["holdsCurrentPosition", "kind", "open", "taskId", "title"],
            "見出しは題名・ID・開閉・現在地の有無だけを運ぶ。完了も件数も持たない"
        );
        assert_eq!(heading["kind"], "task");
        assert!(heading["open"].is_boolean());
        assert!(heading["taskId"].is_string());
        // **開閉とは別の欄である。** 一つにまとめると、閉じたタスクが現在地を抱えて
        // いることを表せない — この欄が要る理由そのものが消える。
        assert!(heading["holdsCurrentPosition"].is_boolean());
        assert_ne!(heading["holdsCurrentPosition"], heading["open"]);

        let step = serde_json::to_value(DisclosureRow::Step {
            step_id: "0198f0e0-0000-7000-8000-000000000000".to_string(),
            content: "下書きを書く".to_string(),
            completed: true,
            current: false,
        })
        .expect("直列化できる");
        assert_eq!(
            step.as_object()
                .expect("オブジェクトである")
                .keys()
                .collect::<Vec<&String>>(),
            vec!["completed", "content", "current", "kind", "stepId"],
            "連番・時刻・メモ・件数に由来する欄を足さない (AD-15)"
        );
        assert_eq!(step["kind"], "step");
        assert!(step["completed"].is_boolean());
        assert!(step["current"].is_boolean());
        assert!(step["stepId"].is_string());
    }

    /// 選択の対象は**ステップ**の ID である。**そのまま送り返せば復元できる。**
    ///
    /// 運ぶ形が壊れていれば、一覧の Enter が毎回「行を特定できない」で落ちる。
    #[test]
    fn a_listed_step_id_round_trips_back_into_the_domain() {
        let state = a_state_with_two_tasks_and_five_steps();
        let surface = disclosure_of(Some(&state), None);
        let DisclosureRow::Step {
            step_id: listed, ..
        } = &surface.rows[4]
        else {
            panic!("5 行目はステップの行である (見出し 2 行 + 開いたタスクの 3 ステップ)")
        };

        let parsed = step_to_select(listed).expect("一覧が運んだ ID はそのまま読み戻せる");

        assert_eq!(parsed.to_string(), *listed);
        assert_eq!(
            parsed,
            state.tasks()[1].steps()[2].id(),
            "読み戻した ID はコアが持つステップそのものを指す"
        );
    }

    /// **一覧の要求も TS → Rust の契約である。** 綴りを変えると、見出しの確定が
    /// 毎回引数の復元で落ちる。
    #[test]
    fn the_disclosure_request_keeps_its_wire_contract() {
        let opened: DisclosureRequest =
            serde_json::from_str(r#"{"openTaskId":"0198f0e0-0000-7000-8000-000000000000"}"#)
                .expect("フロントが送る形で復元できる");
        assert_eq!(
            opened,
            DisclosureRequest {
                open_task_id: Some("0198f0e0-0000-7000-8000-000000000000".to_string()),
            }
        );

        // 面を開いた時点の要求。`null` は**現在地**のタスクである。
        let initial: DisclosureRequest =
            serde_json::from_str(r#"{"openTaskId":null}"#).expect("復元できる");
        assert_eq!(initial.open_task_id, None);

        // 欄が無い要求も `null` と同じである。
        let empty: DisclosureRequest = serde_json::from_str("{}").expect("復元できる");
        assert_eq!(empty.open_task_id, None);

        // **受け付ける綴りは camelCase の一つだけである。** 唯一の欄が `Option` である
        // 以上、知らない欄を拒まなければ綴りの取り違えがどこにも現れない — 見出しを
        // 確定しても黙って現在地のタスクが開き続ける。
        assert!(serde_json::from_str::<DisclosureRequest>(
            r#"{"open_task_id":"0198f0e0-0000-7000-8000-000000000000"}"#
        )
        .is_err());
    }

    /// 見出しが運んだ ID は、そのまま送り返せば同じ**タスク**を開く。
    ///
    /// **期待値はコア状態から読む。** 射影の戻り値だけで閉じると、全見出しが同じ ID を
    /// 運んでいても通る。
    #[test]
    fn a_listed_task_id_round_trips_back_into_the_open_task() {
        let state = a_state_with_two_tasks_and_five_steps();
        let surface = disclosure_of(Some(&state), None);
        let DisclosureRow::Task { task_id, .. } = &surface.rows[0] else {
            panic!("1 行目は見出しである")
        };
        assert_eq!(*task_id, task_id_in(&state, 0), "運ばれた ID は第 1 タスク");

        let opened = disclosure_of(Some(&state), Some(task_id));

        assert_eq!(opened_heading(&opened).as_deref(), Some("原稿"));
        assert_eq!(
            listed_steps(&opened),
            vec!["構成を決める".to_string(), "下書きを書く".to_string()]
        );
    }

    /// **TS → Rust の契約。** フロントが送る JSON がそのまま復元できる。
    #[test]
    fn the_selection_request_keeps_its_wire_contract() {
        let request: SelectStepRequest =
            serde_json::from_str(r#"{"stepId":"0198f0e0-0000-7000-8000-000000000000"}"#)
                .expect("フロントが送る形で復元できる");
        assert_eq!(
            request,
            SelectStepRequest {
                step_id: "0198f0e0-0000-7000-8000-000000000000".to_string(),
            }
        );

        // 受け付ける綴りは camelCase の一つだけである。
        assert!(serde_json::from_str::<SelectStepRequest>(
            r#"{"step_id":"0198f0e0-0000-7000-8000-000000000000"}"#
        )
        .is_err());

        // **メモも完了も受け取らない。** 送られてきても無視される — 型に欄が無い。
        let ignored: SelectStepRequest = serde_json::from_str(
            r#"{"stepId":"0198f0e0-0000-7000-8000-000000000000","note":"書いた","declareCompletion":true}"#,
        )
        .expect("余分な欄は落ちる");
        assert_eq!(ignored.step_id, "0198f0e0-0000-7000-8000-000000000000");
    }

    /// 選択の結末も TS 側と 1:1 である。**欄は `moved` の一つだけである。**
    #[test]
    fn the_selection_outcome_keeps_its_wire_contract() {
        assert_eq!(
            serde_json::to_value(SelectStepOutcome { moved: true }).expect("直列化できる"),
            serde_json::json!({ "moved": true })
        );
        assert_eq!(
            serde_json::to_value(SelectStepOutcome { moved: false }).expect("直列化できる"),
            serde_json::json!({ "moved": false })
        );
    }

    /// 読めない ID は、面を閉じずに示される理由になる。
    ///
    /// **コアを要求する前にここを通る。** この関数がコアを取らないことが、入力の不備に
    /// 対して「状態を読み込めていない」と返らないことの担保である。
    #[test]
    fn an_unreadable_row_is_refused_with_a_reason() {
        assert_eq!(
            step_to_select("not-a-uuid").err().as_deref(),
            Some(ROW_UNREADABLE)
        );
        assert_eq!(step_to_select("").err().as_deref(), Some(ROW_UNREADABLE));
        assert!(
            ROW_UNREADABLE.contains("開き直す"),
            "次に何をすればよいかを示す文であること"
        );
    }

    /// **何も書かれていない選択ではイベントを発行しない** (AD-3)。
    ///
    /// 発行すれば、受け手は起きていない変化のために描き直す。
    #[test]
    fn only_a_move_announces_the_current_position() {
        assert!(announces_a_move(SelectStepOutcome { moved: true }));
        assert!(!announces_a_move(SelectStepOutcome { moved: false }));
    }
}
