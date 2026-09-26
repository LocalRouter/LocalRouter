//! Model-kind classification from GGUF metadata (never from names).

use serde::{Deserialize, Serialize};

use crate::gguf::GgufSummary;

/// What a GGUF file is for.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    /// A causal LM with a chat template.
    Chat,
    /// A causal LM without a chat template.
    Completion,
    /// A text-embedding model (pooled output or non-causal attention).
    Embedding,
    /// A cross-encoder reranker (`pooling_type == rank` or `cls.*` tensors).
    Reranker,
    /// A multimodal projector (`mmproj`, architecture `clip`).
    Projector,
    /// A LoRA / control-vector adapter.
    Adapter,
    /// Not a model llama.cpp can run: no tokenizer, as in image or video
    /// diffusion models packed as GGUF.
    Unsupported,
}

/// llama.cpp `LLAMA_POOLING_TYPE_RANK`.
const POOLING_RANK: u32 = 4;

/// Classify a model from its header summary and `general.type`, in this
/// order: projector (`clip` architecture) → adapter (`general.type ==
/// adapter`) → unsupported (no tokenizer) → reranker (`pooling_type == 4` or `cls.*` tensors) → embedding
/// (`pooling_type ∈ {1,2,3}` or `attention.causal == false`) → chat (has a
/// chat template) → completion.
pub fn classify(summary: &GgufSummary, general_type: Option<&str>) -> ModelKind {
    if summary.is_projector || summary.architecture.as_deref() == Some("clip") {
        return ModelKind::Projector;
    }
    if general_type.is_some_and(|t| t.eq_ignore_ascii_case("adapter")) {
        return ModelKind::Adapter;
    }
    if !summary.has_tokenizer {
        return ModelKind::Unsupported;
    }
    if summary.pooling_type == Some(POOLING_RANK) || summary.has_cls_tensors {
        return ModelKind::Reranker;
    }
    if matches!(summary.pooling_type, Some(1..=3)) || summary.causal == Some(false) {
        return ModelKind::Embedding;
    }
    if summary.has_chat_template {
        return ModelKind::Chat;
    }
    ModelKind::Completion
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gguf::test_support::GgufBuilder;
    use crate::gguf::{parse_header, GgufSummary};

    fn kind(b: GgufBuilder) -> ModelKind {
        let h = parse_header(&b.build()).unwrap();
        classify(&GgufSummary::from_header(&h), h.general_type())
    }

    #[test]
    fn projector() {
        let b = GgufBuilder::new()
            .str("general.architecture", "clip")
            .str("general.type", "mmproj");
        assert_eq!(kind(b), ModelKind::Projector);
    }

    #[test]
    fn adapter() {
        let b = GgufBuilder::decoder("llama")
            .str("general.type", "adapter")
            .str("adapter.type", "lora");
        assert_eq!(kind(b), ModelKind::Adapter);
    }

    #[test]
    fn reranker_by_pooling_and_by_cls_tensor() {
        let b = GgufBuilder::decoder("bert").u32("bert.pooling_type", 4);
        assert_eq!(kind(b), ModelKind::Reranker);
        let b = GgufBuilder::decoder("xlm-roberta")
            .u32("xlm-roberta.pooling_type", 2)
            .tensor("cls.output.weight");
        assert_eq!(kind(b), ModelKind::Reranker);
    }

    #[test]
    fn embedding_by_pooling_and_by_non_causal() {
        for p in 1..=3 {
            let b = GgufBuilder::decoder("bert").u32("bert.pooling_type", p);
            assert_eq!(kind(b), ModelKind::Embedding, "pooling {p}");
        }
        let b = GgufBuilder::decoder("nomic-bert").bool("nomic-bert.attention.causal", false);
        assert_eq!(kind(b), ModelKind::Embedding);
        // Embedding wins over a chat template.
        let b = GgufBuilder::decoder("qwen3")
            .u32("qwen3.pooling_type", 3)
            .str("tokenizer.chat_template", "{{ x }}");
        assert_eq!(kind(b), ModelKind::Embedding);
    }

    #[test]
    fn chat_and_completion() {
        let b = GgufBuilder::decoder("qwen3").str("tokenizer.chat_template", "{{ x }}");
        assert_eq!(kind(b), ModelKind::Chat);
        let b = GgufBuilder::decoder("llama")
            .u32("llama.pooling_type", 0)
            .bool("llama.attention.causal", true);
        assert_eq!(kind(b), ModelKind::Completion);
    }

    #[test]
    fn diffusion_models_without_a_tokenizer_are_unsupported() {
        // An image diffusion transformer packed as GGUF (Qwen-Image style).
        let b = GgufBuilder::new()
            .str("general.architecture", "qwen_image21")
            .u32("general.file_type", 2)
            .tensor("img_in.weight")
            .tensor("transformer_blocks.0.attn.to_k.weight");
        assert_eq!(kind(b), ModelKind::Unsupported);
    }

    #[test]
    fn never_by_name() {
        let b = GgufBuilder::decoder("llama").str("general.name", "bge-reranker-embedding-mmproj");
        assert_eq!(kind(b), ModelKind::Completion);
    }

    #[test]
    fn serde_names() {
        assert_eq!(
            serde_json::to_string(&ModelKind::Reranker).unwrap(),
            "\"reranker\""
        );
    }
}
