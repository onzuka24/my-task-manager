# Editorial Review — addendum.md

**Target:** `_bmad-output/planning-artifacts/prds/prd-my-task-manager-2026-09-14/addendum.md`
**Cross-reference source:** `prd.md` (same directory) — read for reference resolution only; not reviewed.
**Lenses:** `structure` (run first), then `prose` (run on top of the structural findings).
**Style guide:** Microsoft Writing Style Guide · **Reader type:** humans
**Word metrics:** 2,564 words total (§1 308 · §2.1 515 · §2.2 240 · §2.3 194 · §3 47 · §3.1 446 · §3.2 152 · §3.3 88 · §4 362 · preamble 101)

**Purpose/audience read:** This document exists to let the architect, UX designer, and epic-decomposition workflows recover *why* the PRD's choices were made and *how* they might be realized, without that material bulking up the PRD itself. Correct this premise before acting on the findings if it is wrong.

**Structure model applied:** Strategic/Context (Pyramid) — the same model as the PRD it accompanies (decision-record content: rejected alternatives, evidence, provenance). Judged against its rules: top-down, MECE grouping, evidence supports arguments rather than leading them.

**Markdown integrity — checked clean.** All `**` runs are balanced on every line; the §2.1 comparison table is well-formed (3 columns, 3 body rows, all rows terminated with `|`); all `「」`, `()`, `（）` pairs balance; no trailing whitespace; no ideographic spaces. The nested-bold and table concerns raised in the brief did not reproduce. Two *cosmetic* bold inconsistencies inside the table are logged as P7 and P8.

**Severity scale:** High = a downstream reader is actively misled or the document contradicts the PRD · Medium = comprehension or resolvability is impaired · Low = consistency and polish.

---

## Section ownership verdict (addendum vs. PRD)

All four top-level sections belong in an addendum, and none should move into the PRD:

| Section | Verdict |
|---|---|
| §1 課題の理論的裏付け | PRESERVE — research grounding; correctly excluded from the PRD. |
| §2 検討したが採用しなかった選択肢 | PRESERVE — rejected-alternative rationale; the canonical addendum payload. |
| §3 実現手段に関する下流への申し送り | PRESERVE *as a section* — but §3.1 currently re-states PRD material rather than adding depth. See S2. |
| §4 設計原則の由来 | PRESERVE — provenance; correctly excluded from the PRD. |

The problem is not *which* sections are present; it is that three passages re-state the PRD instead of going deeper (S2, S4, S10), and one contradicts it (S1).

---

## Findings — structure lens

| # | Severity | Location | Current text | Replacement / disposition | Tag |
|---|---|---|---|---|---|
| S1 | **High** | §3.3 強い提示形式 (line 56) — full section | `FR-14 の「選択するまで消えない」と §7.2 の「入力先を奪ってはならない」は緊張関係にある。両立の手段は未決定であり、未解決の問い 2 として PRD 本体に記載した。v2 の UX 設計における主要課題となる。` | `FR-15 および FR-14 の「選択肢のいずれかが選ばれるまで介入は消えない」と §7.2 の「入力先を奪ってはならない」は緊張関係にある。両立の手段は未決定であり、§10 OQ-2 として PRD 本体に記載した。FR-15 が v1 スコープであるため、これは v1 の UX 設計における課題である。` | QUESTION |
| S2 | **High** | §3.1 プラットフォーム依存の境界 (lines 44 and 46) | Line 44 opener: `OS 固有の実装を要する点は四つである — FR-1 (グローバルホットキーの捕捉)、FR-3 (ログイン時の自動起動)、FR-15 (最前面での強い提示と入力先の非奪取)、FR-11 (**観測**)。` — and the whole of line 46: `**当初「v1 に FR-11 が含まれないためプラットフォーム決定は v1 の着手を阻害しない」と記載していたが、これは誤りであり撤回する。** FR-1、FR-3、FR-15 はいずれも v1 スコープであり、かつ OS 固有の統合を要する。したがってプラットフォームの決定は v1 の実装着手に先行して必要である (§10 OQ-1)。` | Collapse both into one pointer and keep only what the PRD does not already hold. Replace line 44's opener with: `OS 固有の実装を要する四つの境界は §8 可搬性に列挙済みである。このうち最も重いのは FR-11 である。` — and delete line 46 entirely (the retraction and its reasoning are already carried, in near-identical wording, by PRD §10 OQ-1 and §8 可搬性). | CONDENSE + CUT (~150 words) |
| S3 | Medium | Preamble (line 3), end of paragraph | `PRD が「何を」を定義するのに対し、本書は「なぜその選択に至ったか」と「どう実現しうるか」を記録する。` | `PRD が「何を」を定義するのに対し、本書は「なぜその選択に至ったか」と「どう実現しうるか」を記録する。本書中の § および FR-n、OQ-n、SM-n は、断りのない限り PRD 本体の節・番号を指す。` | QUESTION (scaffolding) |
| S4 | Medium | §2.1 観測方式 (line 24), first three sentences | `この採用根拠が要求として具体化されているかは点検を要した。当初 §4.5 は「宣言されなかった**切り替え**および固着状態を推定する」と述べながら、配下の FR にはどちらの能力も存在しなかった — すなわち最大コストの技術選定を正当化する能力が要件化されていなかった。現在は FR-20 (**固着**の検出) と FR-21 (未宣言の**切り替え**の推定) として明示されている。` | `この採用根拠を要件として担保するのが FR-20 (**固着**の検出) と FR-21 (未宣言の**切り替え**の推定) である。` — then keep the existing final sentence unchanged. The cut material narrates an audit performed on the PRD; the revisit trigger that follows it is the content downstream actually needs. | CONDENSE (~120 words) |
| S5 | Medium | §2 — schema mismatch across §2.1 / §2.2 / §2.3 | §2.1 presents options as a 3-column table (`\| 選択肢 \| 内容 \| 判断 \|`); §2.2 and §2.3 present the identical content type as bold-lead bullets. | Use one schema for all three. Recommended: convert §2.2 and §2.3 to the same `\| 選択肢 \| 内容 \| 判断 \|` table as §2.1, so a reader scanning rejected alternatives reads one shape throughout. (Converting §2.1 to bullets is the equally valid inverse; pick one.) | MERGE (schema) |
| S6 | Medium | §4 設計原則の由来 (lines 62 and 63) — the bolded principle names | Item 1: `**一度に一つだけ見せる**` · Item 2: `**タスクは順序を持つ工程である**` | Item 1: `**一度に見せるのは「次の一手」だけ。**` · Item 2: `**タスクは平坦な項目ではなく、順序を持つ工程である。**` — the section announces itself as the provenance of PRD §1「設計上の三つの賭け」, so the names must match PRD §1 verbatim or downstream cannot match them by name. (Item 3 already matches.) | QUESTION |
| S7 | Medium | §1 課題の理論的裏付け (line 7) | `Sophie Leroy による **attention residue** (注意残余) として研究されている概念と一致する。` | Add a resolvable citation — a section whose entire job is research grounding must be checkable: `Sophie Leroy が **attention residue** (注意残余) として報告した概念と一致する (Leroy, S. "Why is it so hard to do my work?", *Organizational Behavior and Human Decision Processes*, 2009)。` Verify the reference before committing it. | QUESTION |
| S8 | Medium | §3.3 (line 56) | `未解決の問い 2 として PRD 本体に記載した` | `§10 OQ-2 として PRD 本体に記載した` — every other cross-reference in the document uses the ID form (`§10 OQ-1`, `§6.3`, `§7.3`); spelling this one out makes it the only reference a reader cannot resolve by search. (Folded into S1's replacement text above; listed separately so it is not lost if S1 is rejected.) | QUESTION |
| S9 | Low | §2 検討したが採用しなかった選択肢 (line 14) — heading carries 0 words before §2.1 | `## 2. 検討したが採用しなかった選択肢` followed immediately by `### 2.1 観測方式` | `## 2. 検討したが採用しなかった選択肢` + blank line + `以下は PRD の主要な設計判断について、採用しなかった案とその理由を記録する。下流で同じ案が再提案された際の判断材料として用いること。` — §3 has such a lead-in; §2 is the only level-2 section without one. | QUESTION |
| S10 | Low | §4 (lines 62 and 64) — the rationale clauses | Line 62: `肥大化したプレーンテキスト TODO リストが未来のタスクを大量に可視化し、目の前の一手への集中を奪って放棄に至った経験に由来する。` · Line 64: `書き留めること自体は苦ではないという証言に由来する。` | Both restate PRD §1's own rationale before reaching the new point. Condense to the provenance marker only — line 62: `放棄されたプレーンテキスト TODO リストの経験に由来する。` · line 64: `「書き留めること自体は苦ではない」というユーザーの証言に由来する。` — and keep each item's following sentence, which carries the value the PRD lacks. | CONDENSE (~60 words) |
| S11 | Low | Line 1 — document has no YAML frontmatter | File opens directly with `# Addendum — my-task-manager` | Prepend, mirroring `prd.md` so the pair is tracked together: `---` / `title: my-task-manager — addendum` / `status: draft` / `created: 2026-09-14` / `updated: 2026-09-15` / `---` | QUESTION |
| S12 | Low | Preamble (line 3) | `下流の設計 (アーキテクチャ、UX、エピック分解)` | `下流のワークフロー (アーキテクチャ設計、UX 設計、エピック/ストーリー分解)` — matches PRD §0's wording for the same list. | QUESTION |
| S13 | Low | §3.1 (line 44) | `FR-15 (最前面での強い提示と入力先の非奪取)` | `FR-15/§7.2 (最前面での強い提示と入力先の非奪取)` — PRD §8 attributes this boundary to both; the 非奪取 half lives in §7.2, not in FR-15. (Moot if S2 is accepted, which deletes this enumeration.) | QUESTION |

**Cross-reference audit.** Every `§`, `FR-n`, `OQ-n`, and `SM-n` in the addendum was resolved against `prd.md`. All resolve correctly except: **S1** (§3.3 says OQ-2 is a v2 concern and cites FR-14 alone; PRD §10 marks OQ-2 `[v1]` and cites FR-15 first, because FR-15 is in v1 scope per §6.1) and **S8** (OQ-2 referred to by prose name instead of ID). Verified correct: FR-7/FR-8 (§1), SM-2 (§1), §6.3 + FR-15 degradation (§2.1 table), §4.5 + FR-20 + FR-21 (§2.1), §7.3 + FR-10 (§2.2), §10 OQ-1 + §8 (§3.1), FR-3 + §8 + §7.1 + §7.3 (§3.2), §7.2 (§3.3), §1 設計上の三つの賭け (§4). One quoted passage is inaccurate — see P3.

---

## Findings — prose lens

*Run on top of the structural findings above. Passages tagged CUT in S2 and S4 are skipped. P1 and P3 attach to text that survives S4's condensing.*

**Voice and style noted for preservation:** terse declarative 常体 (である/する); spaced em dash `—` as the standard clause separator (13 uses, consistently spaced); half-width space around Latin script and numerals; PRD glossary terms bolded on use (**観測**, **切り替え**, **固着**); the English loanwords `capture` / `grooming` / `residue` are deliberate — they are load-bearing terms carried from PRD §1 and §3 and must NOT be translated. None of these are flagged below.

| # | Severity | Location | Current text | Replacement text | Change |
|---|---|---|---|---|---|
| P1 | Medium | §1 (line 12), final clause | `検証されたことを意味しない — SM-2 が検証すべき対象である。` | `検証されたことを意味しない — これを検証するのが SM-2 である。` | Reverses an inverted subject. As written, `SM-2 が…対象である` says SM-2 is the thing to be verified; PRD §9 defines SM-2 as the *metric that does* the verifying of the core hypothesis. |
| P2 | Medium | §1 (line 7) | `Sophie Leroy による **attention residue** (注意残余) として研究されている概念と一致する。` | `Sophie Leroy が **attention residue** (注意残余) として報告した概念と一致する。` | `A による X として研究されている概念` stacks two modifiers on one noun and leaves the agent ambiguous. (If S7 is accepted, apply this wording inside S7's replacement.) |
| P3 | Medium | §2.1 (line 24), quoted passage | `当初 §4.5 は「宣言されなかった**切り替え**および固着状態を推定する」と述べながら` | `当初 §4.5 は「ユーザーが宣言しなかった**切り替え**および固着状態を推定する」と述べながら` | The 「」 marks this as a direct quote of PRD §4.5, but the source reads `ユーザーが宣言しなかった` (active, with agent), not `宣言されなかった` (passive). Quote the source exactly. (Moot if S4 is accepted, which cuts this sentence.) |
| P4 | Medium | §3 intro (line 40) | `PRD 本体は capability のみを規定し、以下は意図的に記載していない。` | `PRD 本体は能力 (capability) のみを規定し、以下は意図的に記載していない。` | `capability` appears nowhere in `prd.md` and is not a glossary term, so it is an undefined English word rather than a carried term — unlike `capture`/`grooming`. The addendum itself already uses 能力 for this concept twice in §2.1 (`配下の FR にはどちらの能力も存在しなかった`). Gloss it on first use, or drop to plain 能力. |
| P5 | Medium | §2.2 (line 30) | `義務化の risk は FR-10 (未作成時の縮退・非督促) で緩和する。` | `義務化のリスクは FR-10 (未作成時の縮退・非督促) で緩和する。` | Bare Latin-script `risk` in running Japanese, where the document otherwise uses 懸念/懸案/不安 for the same idea (§2.1 `プライバシー懸案`, §2.3 `懸念が強い`, §2.3 `喪失不安`). Note: `prd.md` §6.1 has the same habit (`誤発火の risk`); fixing both keeps the pair consistent, but only the addendum is in scope here. |
| P6 | Low | §3.1 (line 44) | `macOS では Accessibility および画面収録に相当する権限が、` | `macOS ではアクセシビリティおよび画面収録に相当する権限が、` | Two items in one coordinated list, one in Latin script and one in Japanese. macOS ships both permission names in Japanese (アクセシビリティ / 画面収録); matching scripts removes the asymmetry. |
| P7 | Low | §2.1 table (line 22), third cell | `**採用。** 宣言されなかった切り替えと固着の双方を検出できる唯一の方式。` | `**採用。** 宣言されなかった**切り替え**と**固着**の双方を検出できる唯一の方式。` | 切り替え and 固着 are bolded as glossary terms everywhere else in the document (including line 24, ten lines later) but left plain here. |
| P8 | Low | §2.1 table (line 22) vs lines 20–21, third cell | Line 22 opens `**採用。**`; lines 20 and 21 open `不採用。` with no bold. | Either bold all three verdicts (`**不採用。**` / `**不採用。**` / `**採用。**`) or none. If bolding is kept, exclude the 句点: `**採用**。` | The verdict column mixes bolded and unbolded judgments, and the one bolded case swallows its full stop into the emphasis. |
| P9 | Low | §3.1 (line 44) | `両者に共通の抽象は実質的に存在しない。` | `両者に共通の抽象化レイヤは実質的に存在しない。` | 抽象 as a bare noun reads as the adjective stem; 抽象化レイヤ names the thing the architect is being told does not exist. |
| P10 | Low | §3.1 (line 48) | `二つの制約を同時に満たす必要がある — §8 の待機時メモリ 100MB 未満という上限と、v2 で FR-11 の**観測**を実装可能であること。` | `二つの制約を同時に満たす必要がある — §8 の待機時メモリ 100MB 未満という上限を守ること、および v2 で FR-11 の**観測**を実装可能であること。` | The two coordinated items are grammatically mismatched (noun phrase `上限` vs. nominalized clause `…であること`). Parallel form makes the pair scan as one list. |
| P11 | Low | §2.2 (line 30) | `**朝のリチュアル (採用)** — 交差検出に必要な事前の期待値を与える唯一の方式。` | `**朝の計画作成 (採用)** — 交差検出に必要な事前の期待値を与える唯一の方式。` | Consider: `リチュアル` is introduced only here and appears in neither the PRD glossary (§3) nor anywhere else; PRD §3 explicitly forbids introducing synonyms. FR-9 names this 本日計画の作成. If the ritual connotation is deliberate, gloss it instead: `**朝のリチュアル (本日計画の作成・採用)**`. |
| P12 | Low | §2.3 (line 34) | `放棄されたプレーンテキスト TODO リストと同じ結果 (罪悪感の山) を、手順を増やしただけで再現する懸念が強い。` | `放棄されたプレーンテキスト TODO リストと同じ結果 (罪悪感の山) を、手順を増やしただけで再現することになる。` | `結果を…再現する懸念が強い` hedges a stated verdict (`不採用。`) and separates 結果を from its verb by a long insertion. The direct form matches the decisive register of the two sibling bullets. |
| P13 | Low | §3.2 (line 52) | `これは書き込みのタイミングに対する制約であり (切り替えの完了時点で確定していること)、記憶方式の選択そのものには制約を置いていない。` | `これは書き込みのタイミング (**切り替え**の完了時点で確定していること) に対する制約であり、記憶方式の選択そのものには制約を置いていない。` | The parenthetical defines タイミング but sits after 制約であり, so it reads as gloss on the wrong noun. Also bolds **切り替え** per the document's glossary-term convention. |
| P14 | Low | §2.1 (line 24), final sentence | `この二つが v2 で実装されないのであれば、**観測**の採用根拠自体が失われ、タイマー + アイドル検出への降格を再検討すべきである。` | `この二つが v2 で実装されないのであれば、**観測**の採用根拠自体が失われる。その場合はタイマー + アイドル検出への降格を再検討すること。` | Splits a three-clause chain and turns the trailing recommendation into the imperative form the addendum uses elsewhere for downstream instructions (§3 `入力として扱うこと`, §3.1 `構成とすること`, §4 `判断すること`). |
| P15 | Low | §2.2 (lines 28–29) vs §2.3 (lines 34–35) | §2.2: `…利点があるが不採用。` (verdict buried mid-sentence, twice) · §2.3: `— 不採用。…` (verdict leads, twice) | Lead with the verdict in both, matching §2.1's 判断 column and the Pyramid model: line 28 → `**カレンダー連携** — 不採用。二重管理を避けられる利点はあるが、ユーザーの一日がカレンダー上に存在することが前提となり、また外部サービス依存が §7.3 のコスト制約と衝突する。` · line 29 → `**事後記録型 (計画を立てず実績から構築)** — 不採用。朝の義務を課さない利点はあるが、交差検出には「いつ次へ移るべきか」の事前の期待値が不可欠であり、事後記録では原理的に交差を検出できない。` | Front-loads the decision so the section scans uniformly. |

---

## Summary

**Total recommendations: 28** — 13 structure (2 High, 6 Medium, 5 Low) and 15 prose (5 Medium, 10 Low).

**Estimated reduction if every structure recommendation is accepted:** ~330 of 2,564 words (~13%). Concentrated in §3.1 (S2, ~150 words), §2.1 (S4, ~120 words), and §4 (S10, ~60 words); S3, S9, S11, and S12 add back roughly 60 words of scaffolding, for a net reduction near 10%. No length target was provided.

**The two findings that matter most:** S1 is the only place where the addendum *contradicts* the PRD — it tells the UX designer that the strong-presentation problem is a v2 concern when PRD §10 marks OQ-2 `[v1]` and §7.2 states it is a v1 design task. S2 is the largest true redundancy: §3.1's boundary enumeration and platform-decision retraction are both already carried by PRD §8 and §10 OQ-1, in near-identical wording, which directly undercuts the §3 promise that `以下は意図的に記載していない`.

**Comprehension trade-offs.** S2's deletion removes a self-contained restatement of the platform argument, so a reader of the addendum alone will have to open the PRD at §8/§10 — acceptable, because the addendum's stated contract is to *supplement* the PRD, not to stand alone. S4 removes the narrative of how the FR-20/FR-21 gap was found; if that audit trail has value as a decision record, keep one sentence of it rather than the three. S10's condensing is the one place where brevity costs a little warmth — the full failure story is more vivid than the pointer — so reject S10 if §4 is meant to be readable on its own as the principles' origin story.

**No finding challenges a product decision.** Content is sacrosanct; every row above concerns organization, cross-reference accuracy, or expression only.
