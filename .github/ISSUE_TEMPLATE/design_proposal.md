---
name: Design proposal
about:
  Suggest a config schema change, new command surface, or architectural shift
title: 'design: '
labels: kind:design
---

## Motivation

<!-- What problem does this solve? Why now? -->

## Proposed change

<!-- Config schema, CLI surface, struct shapes, code sketches. -->

```toml
# proposed config.toml surface
```

## Alternatives

<!-- Sub-bullet alternatives + why rejected. -->

## Migration cost

<!-- Breaking change? Impact on existing configs, key binds and running servers. -->

## Stability impact

- Breaks `config.toml` schema (existing configs stop parsing)? [ ]
- Changes a default key bind? [ ]
- Changes the client/server protocol (`PROTOCOL_VERSION` bump)? [ ]
- Changes a public crate API? [ ]
- Changes the resurrect save format (version bump needed)? [ ]
