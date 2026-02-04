#!/usr/bin/env python3
"""
Debug script to examine internal embeddings and see if Candle might have issues.
"""

import sys
sys.path.insert(0, '../RouteLLM')

import numpy as np
import torch
from transformers import AutoModelForSequenceClassification, AutoTokenizer

def main():
    print("Loading RouteLLM BERT model...")

    checkpoint = "routellm/bert_gpt4_augmented"

    model = AutoModelForSequenceClassification.from_pretrained(
        checkpoint,
        num_labels=3,
        output_hidden_states=True  # Get hidden states
    )
    tokenizer = AutoTokenizer.from_pretrained(checkpoint)

    model.eval()

    prompts = [
        "Hello!",
        "Find the degree for the given field extension Q(sqrt(2), sqrt(3), sqrt(18)) over Q.\nA. 0\nB. 4\nC. 2\nD. 6\nAnswer:",
    ]

    print("\n=== Token Analysis ===\n")

    for prompt in prompts:
        inputs = tokenizer(prompt, return_tensors="pt", padding=True, truncation=True)

        print(f"Prompt: {prompt[:60]}...")
        print(f"  Input IDs shape: {inputs['input_ids'].shape}")
        print(f"  Input IDs (first 20): {inputs['input_ids'][0][:20].tolist()}")
        print(f"  Attention mask: {inputs['attention_mask'][0].tolist()}")

        # Note: RoBERTa doesn't use token_type_ids, but let's check
        if 'token_type_ids' in inputs:
            print(f"  Token type IDs: {inputs['token_type_ids'][0][:20].tolist()}")
        else:
            print("  Token type IDs: NOT USED (RoBERTa style)")

        with torch.no_grad():
            outputs = model(**inputs)

            # Get logits
            logits = outputs.logits.numpy()[0]
            print(f"  Logits: {logits}")

            # Get CLS embedding (first token of last hidden state)
            hidden_states = outputs.hidden_states[-1]  # Last layer
            cls_embedding = hidden_states[0, 0, :10].numpy()  # First 10 dims of CLS
            print(f"  CLS embedding (first 10): {cls_embedding}")

        print()

    # Check tokenizer config
    print("\n=== Tokenizer Config ===")
    print(f"Vocab size: {tokenizer.vocab_size}")
    print(f"Model max length: {tokenizer.model_max_length}")
    print(f"Pad token ID: {tokenizer.pad_token_id}")
    print(f"CLS token ID: {tokenizer.cls_token_id}")
    print(f"SEP token ID: {tokenizer.sep_token_id}")

    # Check model config
    print("\n=== Model Config ===")
    config = model.config
    print(f"Vocab size: {config.vocab_size}")
    print(f"Hidden size: {config.hidden_size}")
    print(f"Num hidden layers: {config.num_hidden_layers}")
    print(f"Num attention heads: {config.num_attention_heads}")
    print(f"Max position embeddings: {config.max_position_embeddings}")
    print(f"Type vocab size: {config.type_vocab_size}")

if __name__ == "__main__":
    main()
