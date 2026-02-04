#!/usr/bin/env python3
"""Check how HuggingFace handles token_type_embeddings for XLM-RoBERTa"""

from transformers import AutoModelForSequenceClassification, AutoTokenizer, AutoConfig
import torch

checkpoint = "routellm/bert_gpt4_augmented"

config = AutoConfig.from_pretrained(checkpoint)
print(f"Config type_vocab_size: {config.type_vocab_size}")

model = AutoModelForSequenceClassification.from_pretrained(checkpoint, num_labels=3)
tokenizer = AutoTokenizer.from_pretrained(checkpoint)

# Check if model has token_type_embeddings
print("\nModel structure (embeddings):")
for name, param in model.named_parameters():
    if "embed" in name.lower():
        print(f"  {name}: {param.shape}")

# Check what forward() expects
print("\nChecking token_type_embeddings behavior...")

prompt = "Hello!"
inputs = tokenizer(prompt, return_tensors="pt", padding=True, truncation=True)
print(f"Tokenizer output keys: {inputs.keys()}")

# Try with explicit token_type_ids
inputs_with_token_type = inputs.copy()
inputs_with_token_type['token_type_ids'] = torch.zeros_like(inputs['input_ids'])

print(f"\nWith token_type_ids=zeros:")
with torch.no_grad():
    out1 = model(**inputs)
    out2 = model(**inputs_with_token_type)

print(f"  Without token_type_ids: logits = {out1.logits.numpy()[0]}")
print(f"  With token_type_ids=0: logits = {out2.logits.numpy()[0]}")

# Check if they're the same
diff = torch.abs(out1.logits - out2.logits).max().item()
print(f"  Max difference: {diff}")

# Check the actual embedding weight shape
if hasattr(model, 'roberta'):
    emb = model.roberta.embeddings
    if hasattr(emb, 'token_type_embeddings'):
        print(f"\ntoken_type_embeddings weight shape: {emb.token_type_embeddings.weight.shape}")
        print(f"token_type_embeddings weight values (first 5 dims): {emb.token_type_embeddings.weight[0, :5]}")
    else:
        print("\nNo token_type_embeddings found in model!")
