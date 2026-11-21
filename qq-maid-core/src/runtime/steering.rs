//! 回合内用户插话（steering）注册表。
//!
//! Gateway 在同一 scope 已有 Agent 回合运行时，会把新的普通文本消息优先交给
//! 当前回合，而不是排队等待。本模块只负责“哪个会话正在跑、还能不能接收插话”，
//! 不理解平台协议，也不直接调用模型。

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use qq_maid_llm::agent_loop::AgentInterjectionSource;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tracing::{debug, warn};

use super::tools::message::TurnDelivery;

#[derive(Clone, Default)]
pub struct ActiveTurnRegistry {
    inner: Arc<RegistryInner>,
}

#[derive(Default)]
struct RegistryInner {
    turns: Mutex<HashMap<String, ActiveTurnEntry>>,
    next_id: AtomicU64,
}

struct ActiveTurnEntry {
    turn_id: u64,
    delivery: Arc<TurnDelivery>,
    sender: UnboundedSender<String>,
}

/// 一个正在运行的 Agent 回合。
///
/// 在 `Drop` 时从注册表移除；回合进入收尾后应先调用
/// [`Self::close`]，让新的插话回落到 Gateway backlog。
pub struct ActiveTurn {
    registry: ActiveTurnRegistry,
    scope_key: String,
    turn_id: u64,
    pub delivery: Arc<TurnDelivery>,
    source: Arc<SteeringSource>,
}

impl ActiveTurn {
    pub fn source(&self) -> Arc<dyn AgentInterjectionSource> {
        self.source.clone()
    }

    /// 拒绝后续插话；已经入队的插话仍会被下一次 drain 取走。
    pub fn close(&self) {
        self.delivery.close();
    }

    /// 取走已经接收但最终没有进入模型输入的插话。
    ///
    /// 回合结束后 Core 必须记录这些文本（例如写入 session 历史），不能直接丢弃。
    pub fn take_unanswered(&self) -> Vec<String> {
        let mut texts = self.source.take_requeued();
        texts.extend(self.source.drain());
        texts
    }
}

impl Drop for ActiveTurn {
    fn drop(&mut self) {
        let mut turns = self
            .registry
            .inner
            .turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if turns
            .get(&self.scope_key)
            .is_some_and(|entry| entry.turn_id == self.turn_id)
        {
            turns.remove(&self.scope_key);
        }
    }
}

impl ActiveTurnRegistry {
    /// 注册当前回合。`max_messages` 是本轮允许的发言总条数。
    pub fn begin_turn(
        &self,
        scope_key: &str,
        max_messages: usize,
        reply_to_message_id: Option<String>,
    ) -> ActiveTurn {
        let turn_id = self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let delivery = Arc::new(TurnDelivery::new(max_messages, reply_to_message_id));
        let (sender, receiver) = unbounded_channel();
        let source = Arc::new(SteeringSource {
            receiver: Mutex::new(receiver),
            delivery: delivery.clone(),
            requeued: Mutex::new(Vec::new()),
        });
        let entry = ActiveTurnEntry {
            turn_id,
            delivery: delivery.clone(),
            sender,
        };
        let mut turns = self
            .inner
            .turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if turns.insert(scope_key.to_owned(), entry).is_some() {
            warn!(
                scope_hash = %scope_hash(scope_key),
                turn_id,
                "同一会话出现并发 Agent 回合，已由新回合接管插话"
            );
        }
        debug!(
            scope_hash = %scope_hash(scope_key),
            turn_id,
            max_messages,
            "已注册 Agent 回合插话入口"
        );
        ActiveTurn {
            registry: self.clone(),
            scope_key: scope_key.to_owned(),
            turn_id,
            delivery,
            source,
        }
    }

    /// 把用户插话投递给当前回合；没有可接收回合时返回 false。
    pub fn try_steer(&self, scope_key: &str, text: &str, message_id: Option<&str>) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        let turns = self
            .inner
            .turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = turns.get(scope_key) else {
            return false;
        };
        if !entry.delivery.is_accepting() {
            return false;
        }
        if entry.sender.send(text.to_owned()).is_err() {
            return false;
        }
        entry.delivery.update_latest_reply_to(message_id);
        debug!(
            scope_hash = %scope_hash(scope_key),
            turn_id = entry.turn_id,
            "已将用户插话注入当前 Agent 回合"
        );
        true
    }
}

struct SteeringSource {
    receiver: Mutex<UnboundedReceiver<String>>,
    delivery: Arc<TurnDelivery>,
    /// 已被取走但未进入模型输入的插话；回合结束后由 Core 记录。
    requeued: Mutex<Vec<String>>,
}

impl SteeringSource {
    fn take_requeued(&self) -> Vec<String> {
        std::mem::take(
            &mut *self
                .requeued
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

impl AgentInterjectionSource for SteeringSource {
    fn drain(&self) -> Vec<String> {
        let mut receiver = self
            .receiver
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut texts = Vec::new();
        while let Ok(text) = receiver.try_recv() {
            if !text.trim().is_empty() {
                texts.push(text);
            }
        }
        texts
    }

    fn requeue(&self, texts: Vec<String>) {
        if texts.is_empty() {
            return;
        }
        self.requeued
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(texts);
    }

    fn close(&self) {
        self.delivery.close();
    }
}

fn scope_hash(scope_key: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    scope_key.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steer_requires_active_turn_and_accepting_state() {
        let registry = ActiveTurnRegistry::default();
        assert!(!registry.try_steer("private:u1", "在吗", None));
        let turn = registry.begin_turn("private:u1", 3, Some("m1".to_owned()));
        assert!(registry.try_steer("private:u1", "好了吗", Some("m2")));
        assert_eq!(
            turn.delivery.latest_reply_to_message_id().as_deref(),
            Some("m2")
        );
        turn.close();
        assert!(!registry.try_steer("private:u1", "还在吗", None));
        let drained = turn.source().drain();
        assert_eq!(drained, vec!["好了吗".to_owned()]);
    }

    #[test]
    fn dropping_turn_removes_registration() {
        let registry = ActiveTurnRegistry::default();
        {
            let _turn = registry.begin_turn("private:u1", 3, None);
            assert!(registry.try_steer("private:u1", "插话", None));
        }
        assert!(!registry.try_steer("private:u1", "插话", None));
    }

    #[test]
    fn unanswered_interjections_are_returned_for_history() {
        let registry = ActiveTurnRegistry::default();
        let turn = registry.begin_turn("private:u1", 3, None);
        assert!(registry.try_steer("private:u1", "还在吗", None));
        // Agent Loop 已取走但没能注入模型输入的插话会退回来源。
        turn.source().requeue(vec!["没处理的消息".to_owned()]);
        turn.close();

        let mut unanswered = turn.take_unanswered();
        unanswered.sort();
        assert_eq!(
            unanswered,
            vec!["没处理的消息".to_owned(), "还在吗".to_owned()]
        );
        // 取走后不重复返回。
        assert!(turn.take_unanswered().is_empty());
    }
}
