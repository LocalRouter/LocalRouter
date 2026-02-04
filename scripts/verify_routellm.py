#!/usr/bin/env python3
"""
Verify RouteLLM Python implementation outputs for comparison with Rust.
Run this from the localrouterai directory.
"""

import sys
sys.path.insert(0, '../RouteLLM')

import numpy as np
import torch
from transformers import AutoModelForSequenceClassification, AutoTokenizer

def main():
    print("Loading RouteLLM BERT model from HuggingFace...")

    checkpoint = "routellm/bert_gpt4_augmented"

    model = AutoModelForSequenceClassification.from_pretrained(checkpoint, num_labels=3)
    tokenizer = AutoTokenizer.from_pretrained(checkpoint)

    model.eval()

    prompts = [
        "What is 2+2?",
        "Hello!",
        "Explain quantum entanglement and its implications for EPR paradox",
        "Write a proof that there are infinitely many prime numbers",
        "Analyze the geopolitical implications of climate change on international relations",
        "What is the capital of France?",
        "Find the degree for the given field extension Q(sqrt(2), sqrt(3), sqrt(18)) over Q.\nA. 0\nB. 4\nC. 2\nD. 6\nAnswer:",
    ]

    print("\n=== Python RouteLLM Win Rates ===\n")
    print(f"{'Prompt (truncated)':<60} | {'Win Rate':>10} | {'Probs':>30}")
    print("-" * 110)

    for prompt in prompts:
        inputs = tokenizer(prompt, return_tensors="pt", padding=True, truncation=True)

        with torch.no_grad():
            outputs = model(**inputs)
            logits = outputs.logits.numpy()[0]

        # Apply softmax (same as Python RouteLLM)
        exp_scores = np.exp(logits - np.max(logits))
        softmax_scores = exp_scores / np.sum(exp_scores)

        # Calculate win_rate the same way Python RouteLLM does:
        # binary_prob = softmax[-2:] (sum of last 2 classes: tie + weak wins)
        # win_rate = 1 - binary_prob (probability strong model wins)
        binary_prob = np.sum(softmax_scores[-2:])
        win_rate = 1 - binary_prob  # This equals softmax_scores[0]

        truncated = prompt[:57] + "..." if len(prompt) > 60 else prompt
        probs_str = f"[{softmax_scores[0]:.4f}, {softmax_scores[1]:.4f}, {softmax_scores[2]:.4f}]"
        print(f"{truncated:<60} | {win_rate:>10.4f} | {probs_str:>30}")

    print("\n=== Interpretation ===")
    print("LABEL_0 = strong model wins  (high = need strong)")
    print("LABEL_1 = tie")
    print("LABEL_2 = weak model wins    (high = weak is fine)")
    print()
    print("win_rate = 1 - (probs[1] + probs[2]) = probs[0]")
    print("Route to strong if win_rate >= threshold")

if __name__ == "__main__":
    main()
