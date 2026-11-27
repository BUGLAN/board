use super::CHUNK_FLUSH;
use serde_json::{json, Value};
use std::time::Instant;

/* ------------------------------ 日志：合并消息片段 ------------------------------ */

/// 连续的同一种消息片段合成一帧。每个会话各攒各的（子 agent 的 sessionId 形如
/// `<主会话>:subagent:<uuid>`）：子 agent 的片段和主 agent 交错时，不把主 agent 的话冲散。
#[derive(Default)]
pub(crate) struct Merger {
    /// （sessionId、片段种类、攒的文字、第一段的时间）；一个会话同时只有一种片段，按开始攒的先后排
    bufs: Vec<(Value, String, String, Instant)>,
}

impl Merger {
    /// 一帧 agent 发来的：返回现在要写进日志的行（片段先攒着）
    pub(crate) fn push(&mut self, v: &Value) -> Vec<String> {
        if let Some((kind, session, text)) = chunk(v) {
            if let Some(i) = self.bufs.iter().position(|(s, _, _, _)| *s == session) {
                if self.bufs[i].1 == kind {
                    self.bufs[i].2.push_str(&text);
                    return vec![];
                }
                // 同一个会话换了种类：先把它之前那段落下来，保持这个会话里的先后
                let line = frame(self.bufs.remove(i));
                self.bufs.push((session, kind, text, Instant::now()));
                return vec![line];
            }
            self.bufs.push((session, kind, text, Instant::now()));
            return vec![];
        }
        // 不是片段：这个会话攒的话先落，再写这一帧；没有 sessionId 的全局帧先落全部
        let session = &v["params"]["sessionId"];
        let mut out = if session.is_null() { self.flush() } else { self.take_session(session) };
        out.push(v.to_string());
        out
    }


    /// 有片段已经攒够 CHUNK_FLUSH
    pub(crate) fn due(&self) -> bool {
        self.bufs.iter().any(|(_, _, _, at)| at.elapsed() >= CHUNK_FLUSH)
    }


    /// 攒够 CHUNK_FLUSH 的片段落下来（按开始攒的先后）
    pub(crate) fn flush_due(&mut self) -> Vec<String> {
        let mut due = vec![];
        let mut keep = vec![];
        for buf in std::mem::take(&mut self.bufs) {
            if buf.3.elapsed() >= CHUNK_FLUSH {
                due.push(frame(buf));
            } else {
                keep.push(buf);
            }
        }
        self.bufs = keep;
        due
    }


    /// 全部落下来（退出、写非协议行之前）
    pub(crate) fn flush(&mut self) -> Vec<String> {
        std::mem::take(&mut self.bufs).into_iter().map(frame).collect()
    }


    /// 先落某个会话攒的片段
    fn take_session(&mut self, session: &Value) -> Vec<String> {
        let mut hit = vec![];
        let mut keep = vec![];
        for buf in std::mem::take(&mut self.bufs) {
            if buf.0 == *session {
                hit.push(frame(buf));
            } else {
                keep.push(buf);
            }
        }
        self.bufs = keep;
        hit
    }
}


/// 一个片段攒成的日志行（和 agent 发来的 `session/update` 同形）
fn frame((session, kind, text, _): (Value, String, String, Instant)) -> String {
    let update = json!({ "sessionUpdate": kind, "content": { "type": "text", "text": text } });
    json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": session, "update": update } }).to_string()
}


/// 是消息片段（文字）时：（种类、sessionId、文字）
pub(super) fn chunk(v: &Value) -> Option<(String, Value, String)> {
    if v["method"].as_str() != Some("session/update") {
        return None;
    }
    let u = &v["params"]["update"];
    let kind = u["sessionUpdate"].as_str().filter(|k| matches!(*k, "agent_message_chunk" | "agent_thought_chunk"))?;
    let text = u["content"]["text"].as_str().filter(|_| u["content"]["type"].as_str() == Some("text"))?;
    Some((kind.to_string(), v["params"]["sessionId"].clone(), text.to_string()))
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn piece(kind: &str, text: &str) -> Value {
        json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s", "update": { "sessionUpdate": kind, "content": { "type": "text", "text": text } } } })
    }


    #[test]
    fn message_pieces_are_merged_into_one_line() {
        let mut m = Merger::default();
        assert!(m.push(&piece("agent_thought_chunk", "想")).is_empty());
        let out = m.push(&piece("agent_message_chunk", "先"));
        assert_eq!(out.len(), 1, "换了种类：先落思考");
        assert!(out[0].contains("\"agent_thought_chunk\"") && out[0].contains("想"));
        assert!(m.push(&piece("agent_message_chunk", "领卡，")).is_empty());
        assert!(m.push(&piece("agent_message_chunk", "再看代码。")).is_empty());
        let tool = json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s", "update": { "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "ls" } } });
        let out = m.push(&tool);
        assert_eq!(out.len(), 2, "攒的话先落，再写工具调用");
        let v: Value = serde_json::from_str(&out[0]).unwrap();
        assert_eq!(v["params"]["update"]["content"]["text"], "先领卡，再看代码。");
        assert_eq!(v["params"]["update"]["sessionUpdate"], "agent_message_chunk");
        assert!(out[1].contains("tool_call"));
        assert!(m.flush().is_empty());
        assert!(!m.due());
    }

    fn piece_of(session: &str, kind: &str, text: &str) -> Value {
        json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": session, "update": { "sessionUpdate": kind, "content": { "type": "text", "text": text } } } })
    }


    /// 子 agent 的片段和主 agent 交错时，不把主 agent 的一句话拆成多段；两个子 agent 各自的话也各是一条（default/main#617）
    #[test]
    fn interleaved_subagent_chunks_do_not_split_messages() {
        let mut m = Merger::default();
        let (main, a, b) = ("main", "main:subagent:a", "main:subagent:b");
        assert!(m.push(&piece_of(main, "agent_message_chunk", "定向测试暴露的是")).is_empty());
        assert!(m.push(&piece_of(a, "agent_thought_chunk", "子 agent 在想")).is_empty());
        assert!(m.push(&piece_of(main, "agent_message_chunk", "三处测试桩")).is_empty());
        assert!(m.push(&piece_of(b, "agent_thought_chunk", "另一个在想")).is_empty());
        assert!(m.push(&piece_of(main, "agent_message_chunk", "问题而非")).is_empty());
        // 子 agent a 从「想」转到「说」：只落它自己的思考，主 agent 的话不受影响
        let one = m.push(&piece_of(a, "agent_message_chunk", "子 agent 说的话"));
        assert_eq!(one.len(), 1, "{one:#?}");
        assert!(one[0].contains("子 agent 在想"));
        // 主会话来一个工具调用：主会话攒的话先落成一条，工具调用跟上
        let tool = json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": main, "update": { "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "ls" } } });
        let out = m.push(&tool);
        assert_eq!(out.len(), 2, "主会话只有一条消息 + 工具调用：{out:#?}");
        let v: Value = serde_json::from_str(&out[0]).unwrap();
        assert_eq!(v["params"]["sessionId"], main);
        assert_eq!(v["params"]["update"]["content"]["text"], "定向测试暴露的是三处测试桩问题而非");
        assert!(out[1].contains("tool_call"));
        // 两个子 agent 各自的话各落一条，互不拼接
        let rest = m.flush();
        assert_eq!(rest.len(), 2, "{rest:#?}");
        let texts: Vec<String> = rest
            .iter()
            .map(|l| {
                let v: Value = serde_json::from_str(l).unwrap();
                assert!(v["params"]["sessionId"].as_str().unwrap().starts_with("main:subagent:"));
                v["params"]["update"]["content"]["text"].as_str().unwrap().to_string()
            })
            .collect();
        assert!(texts.iter().any(|t| t == "子 agent 说的话"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "另一个在想"), "{texts:?}");
        assert!(!texts.iter().any(|t| t.contains("定向测试暴露")), "{texts:?}");
    }
}
