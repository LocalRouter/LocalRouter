# LLM Provider Referral & Affiliate Programs Research

**Date:** 2026-03-09
**Status:** Research complete

## Overview

Research into which LLM API providers supported by LocalRouter offer referral/affiliate programs, their terms, and technical implementation details.

---

## Summary Table

| Provider | Has Program? | Type | Commission/Reward |
|---|---|---|---|
| **OpenAI** | No | -- | -- |
| **Anthropic** | Yes (enterprise) | One-time referral fee | Negotiated per deal |
| **Google/Gemini** | Yes (via GCP) | CJ Affiliate | ~5% + tiered scaling |
| **Mistral** | No | Ambassador (credits only) | Free API credits |
| **Cohere** | No | Enterprise partners only | -- |
| **Groq** | No | Invite-only partner credits | -- |
| **Perplexity** | Yes | Dub Partners affiliate | $3-$20/signup |
| **Together AI** | No | -- | -- |
| **DeepInfra** | No | -- | -- |
| **Cerebras** | Yes | Token referral program | 200K tokens/day (cap 1M) |
| **xAI (Grok)** | No | -- | -- |
| **OpenRouter** | No | -- | -- |
| **Fireworks AI** | No | -- | -- |

---

## Detailed Findings

### 1. OpenAI

**Program:** None
OpenAI does not have a public affiliate or referral program for the API or ChatGPT. They focus on strategic enterprise partnerships rather than affiliate-style programs. They have a partner interest intake form and a limited ChatGPT Plus promotions page, but neither offers commissions or referral credits. Reports suggest they are internally developing referral features, but nothing is live.

### 2. Anthropic

**Program:** Enterprise Referral Partner Program (application-based)
- **Type:** One-time flat fee per referred enterprise customer (not recurring)
- **Commission:** Not publicly disclosed; negotiated case-by-case
- **Requirements:** Must bring enterprise deals Anthropic would not get otherwise; C-level relationships at target companies expected; deals should be outside Anthropic's current sales territories
- **How it works:** Apply via anthropic.com/referral. Terms published March 26, 2025
- **Technical:** No self-serve referral links or API headers. Sales-driven, relationship-based program
- **Alternative:** Referring via AWS Bedrock/Marketplace lets AWS partners earn on total cloud spend
- **Also:** VC Partner Program offering API credits to portfolio companies of selected VCs

### 3. Google (Gemini)

**Program:** Google Cloud Affiliate Program (covers Gemini API indirectly)
- **Type:** Commission-based affiliate program via CJ Affiliate
- **Commission:** ~5% per sale baseline, with tiered commissions that scale with volume. No annual earnings cap
- **Perks:** Affiliates can share a $350 free trial credit offer ($300 standard + $50 bonus)
- **How to join:** Sign up free at CJ Affiliate
- **Technical:** Standard CJ Affiliate tracking links and banners. Dashboard for tracking conversions. Payments via direct deposit or check (net-30)
- **Note:** This is for Google Cloud broadly (not Gemini API specifically). Since the Gemini API is part of Google Cloud / Vertex AI, referrals who use Gemini through Google Cloud would count

### 4. Mistral

**Program:** None
No affiliate or referral program. They have an Ambassador Program providing free API credits for community contributors (requires AI/ML experience and active community participation), and enterprise-level partner integrations with cloud providers, but no commission-based referral mechanism.

### 5. Cohere

**Program:** None (affiliate-style)
Enterprise Partner Program with tiered partnerships (Consulting, Technology, Research/Academic), focused on joint go-to-market and enterprise deployments, not individual affiliate commissions. No self-serve referral links or commission structures.

### 6. Groq

**Program:** Partner Program (invitation-only, no affiliate)
The Groq Partner Program is an exclusive, hand-selected program for scaling companies to get inference credits. Not an affiliate or commission-based program. Eligibility is at Groq's sole discretion.

### 7. Perplexity

**Program:** Yes - Active affiliate program via Dub Partners
- **Type:** Pay-per-action affiliate program
- **Commission:** $3-$20 per signup (varies by country)
- **Consumer referral:** "Give a month, Get a month" of Pro (cap of 12 redemptions)
- **How to join:** Sign up at partners.dub.co/perplexity - free, no fees
- **Technical:** Unique referral links via Dub Partners platform. Dashboard with real-time clicks, conversions, and earnings. 30-day holding period before payout. Payments via bank, PayPal, or crypto
- **Note:** Primarily for Perplexity Pro consumer subscriptions, NOT the Perplexity API specifically

### 8. Together AI

**Program:** None
No affiliate, referral, or commission-based partner program. They have a Startup Accelerator offering up to $50K in credits, but nothing resembling an affiliate program.

### 9. DeepInfra

**Program:** None
No affiliate, referral, or partner program. Straightforward pay-as-you-go API pricing only.

### 10. Cerebras

**Program:** Yes - Referral Program (consumer/developer level)
- **Type:** Token-based referral rewards
- **Reward:** Both referrer and referee receive 200,000 bonus tokens per day
- **Cap:** 1,000,000 tokens total per user. Referrer can earn bonuses for up to 5 successful referrals
- **Eligibility:** US personal account holders only (also available in India, EU, UK, Canada, Australia, Singapore, Japan via international addendum)
- **Technical:** Opt-in via in-product flow to generate a referral link. Personal/non-commercial use only. Cannot publish on public websites or discount sites
- **Also:** API Certification Partner Program for LLM API providers (enterprise-level). Fellows Program for creators with increased rate limits

### 11. xAI (Grok)

**Program:** None
No affiliate, referral, or partner program found.

### 12. OpenRouter

**Program:** None
No affiliate or referral program. OpenRouter does have an App Attribution system where apps sending `HTTP-Referer` and `X-OpenRouter-Title` headers get listed in public rankings/leaderboards, but this provides visibility, not commissions or credits.

### 13. Fireworks AI

**Program:** None (affiliate-style)
Developer Partners Program focused on technical integrations (featured in docs, co-marketing, technical support), but no commission-based affiliate or referral program.

---

## Key Takeaways

1. **Most LLM API providers do not offer affiliate/referral programs.** Only 4 out of 13 have any form of referral mechanism.

2. **The programs that exist are quite different from each other:**
   - **Cerebras** has the most developer-friendly program with automatic token rewards via in-product referral links
   - **Perplexity** has the most traditional affiliate program (via Dub Partners), but targets consumer Pro subscriptions, not API usage
   - **Google Cloud** offers a standard affiliate program via CJ Affiliate that indirectly covers Gemini API usage
   - **Anthropic** has an enterprise-only referral program with negotiated one-time fees

3. **No provider offers API-level referral tracking** (e.g., a referral header in API requests that credits the referrer). The closest is OpenRouter's `HTTP-Referer` header for app attribution, but it provides visibility rather than monetary rewards.

4. **For LocalRouter**, the most technically feasible integration points would be:
   - **Cerebras:** Generate referral links for users signing up for Cerebras accounts
   - **Google Cloud:** Use CJ Affiliate tracking links when directing users to set up GCP/Gemini
   - **Perplexity:** Dub Partners links for Pro signups (not API-relevant)
   - **Anthropic:** Would require a formal enterprise partnership application

## Implications for LocalRouter

The landscape is thin. The only realistic, self-serve affiliate programs relevant to API usage are:
- **Google Cloud (CJ Affiliate)** — could embed tracking links in the provider setup wizard when users create a GCP account
- **Cerebras** — could share referral links, but restricted to personal/non-commercial use and US + select countries

The rest either don't exist, are enterprise-only (Anthropic), or target consumer products not API usage (Perplexity).
