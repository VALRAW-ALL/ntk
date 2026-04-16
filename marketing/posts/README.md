# NTK launch posts

Ready-to-publish posts announcing NTK across three platforms in two
languages. Each file is standalone — copy the body into the platform,
adjust the live metric numbers if they've moved since the post was
written, and hit publish.

| File | Platform | Language | Best sub/channel |
|---|---|---|---|
| `tabnews-pt-br.md`  | [TabNews](https://www.tabnews.com.br/) | pt-BR | "pub" |
| `tabnews-en-us.md`  | TabNews | en-US | "pub" (international audience) |
| `reddit-pt-br.md`   | Reddit   | pt-BR | r/brdev, r/programacao |
| `reddit-en-us.md`   | Reddit   | en-US | r/rust, r/LocalLLaMA, r/ClaudeAI, r/opensource |
| `linkedin-pt-br.md` | LinkedIn | pt-BR | personal feed |
| `linkedin-en-us.md` | LinkedIn | en-US | personal feed |

## Ranking mechanisms baked in

- **Title hook** — first 60–90 chars carry the core promise + a concrete
  number (e.g. "compresses 60–90 %").
- **TL;DR at top** — Reddit / TabNews readers skim. First screenful
  delivers value.
- **Numbers over adjectives** — "up to 92 %" instead of "huge savings".
  Specific numbers rank and build trust.
- **Question as CTA** — each post ends with an open question inviting
  comments, which drives engagement signal on all three platforms.
- **Hashtags / tags** — LinkedIn blocks at the bottom; TabNews tag list
  in the frontmatter; Reddit has no native tags but the body mentions
  the tech stack so the subreddit's own keyword search surfaces it.
- **"Needs contributors" framing** — all posts ask for help, not
  adoration. This both matches the project's real state and draws in
  the readers who open-source platforms reward (contributors, not
  consumers).

## Editing rules when adapting

- Numbers must be **measured**, not claimed. If you can't point at a
  cell in `bench/microbench.csv`, don't use the number.
- Keep the "early-stage" framing. Overselling backfires on Reddit and
  TabNews within hours (the comments section will correct it).
- Never promise features that aren't merged.
