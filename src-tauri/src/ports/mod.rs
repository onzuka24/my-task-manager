//! ポート — コアが外界に要求する契約 (トレイト) を置く層 (AD-1)。
//!
//! ポートはコア側の語彙で書く。実装 (アダプタ) の型がここに現れてはならない。
//!
//! - [`storage`] — 永続化の契約 (AD-4 / AD-5)。実装は `adapters/storage`
//!
//! observation / presentation の契約はまだ存在しない。observation は v2 (AD-9)、
//! presentation は現時点でアダプタがコアを呼ぶ向きしか持たないため、要求する契約が無い。

pub mod storage;
