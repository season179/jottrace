# Command Code "Taste" — Meta-NeuroSymbolic Objective, Decoded

*Notes from reading the formula in Command Code's docs (2026-06-10).*

Sources:
- https://commandcode.ai/docs/markdown/taste/index.md
- https://commandcode.ai/docs/markdown/taste/manage/index.md

## What "Taste" is (per the docs)

- An AI system in Command Code that learns individual developer preferences from
  accept / reject / edit signals, powered by a model called `taste-1`, marketed as a
  "meta neuro-symbolic AI model with continuous reinforcement learning."
- Profiles are portable and shareable via Git-like commands:
  `npx taste push / pull / list / open <package>` (after `cmd login`).
- The docs pages contain **no technical detail** about the training objective —
  the formula appears without explanation.

## The formula

$$
\text{Objective}(\phi) =
\mathbb{E}_{x\sim D_{RL}}\;\mathbb{E}_{y\sim LLM^{NS}_\phi(x)}
\left[ RM_{NS}(x,y) - \beta_{NS}\log\frac{LLM^{NS}_\phi(y|x)}{LLM^{SFT}(y|x)} \right]
+ \gamma_{NS}\;\mathbb{E}_{x\sim D_{pretrain}}\log LLM^{NS}_\phi(x)
$$

This is the **InstructGPT RLHF objective** (Ouyang et al. 2022, "PPO-ptx", eq. 2)
with every symbol given an `NS` subscript. φ are the policy model's parameters,
and training **maximizes** this objective.

### Term 1 — reward with a KL leash

- Sample prompt `x` from the RL dataset (your coding context), sample response `y`
  from the current policy `LLM^NS_φ`.
- `RM_NS(x, y)`: a reward model scores the response. In the taste context it would be
  trained from accept/reject/edit signals, so reward ≈ "matches your taste."
- `β_NS · log(policy / SFT)`: per-sample KL penalty against the frozen SFT baseline.
  Limits drift from the supervised model so the policy can't game the reward model
  (reward hacking). β controls how tight the leash is.

### Term 2 — pretraining anchor

- `γ_NS · E[log LLM^NS_φ(x)]` over pretraining data: plain language-modeling loss
  mixed in with weight γ.
- Prevents catastrophic forgetting of general coding/language ability while the
  model is pushed toward learned preferences. This is the "ptx" in PPO-ptx.

### One-line intuition

> Maximize my taste-reward, without straying too far from the safe baseline model,
> while still remembering everything from pretraining.

## Honest assessment

Nothing in the equation is "meta" or "neuro-symbolic" — it is the standard 2022
RLHF objective verbatim; the `NS` subscripts are branding, not math. If symbolic
reasoning exists in the system, it would have to live inside `RM_NS` (e.g.,
rule/lint-like predicates contributing to reward) or in data construction —
neither is specified by the formula or the docs.

## Jottrace implementation

Jottrace produces the `D_RL` training rows this formula samples from: labeled
`(context, proposal, outcome)` triples derived deterministically from preserved
Claude sessions. The data-extraction layer is implemented as `jottrace taste`
with migrations `010`–`013`, present-at-session-end outcome labeling, snapshot
sidecar resolution, and JSONL export. Design detail and risk coverage:
`notes/taste-extraction-plan.md` (status: IMPLEMENTED).

## Reference

- Ouyang et al., *Training language models to follow instructions with human
  feedback* (InstructGPT), 2022 — https://arxiv.org/abs/2203.02155
- Jottrace taste extraction: `notes/taste-extraction-plan.md`, `docs/design.md`
