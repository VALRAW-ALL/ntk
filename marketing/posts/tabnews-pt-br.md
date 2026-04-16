---
platform: tabnews
language: pt-BR
channel: pub
suggested_tags: [rust, llm, opensource, claude, ia, open-source, desenvolvimento, cli]
seo_keywords:
  - compressor de contexto LLM
  - proxy de compressão para Claude Code
  - NTK neural token killer
  - rust cli token
  - hook PostToolUse Claude Code
title_char_budget: 150
recommended_title: "NTK: um daemon em Rust que comprime 60–90 % da saída de comandos antes de chegar no Claude Code"
---

<!--
Este arquivo é apenas o corpo do post. Cole o título do frontmatter
no campo de título do TabNews e cole o corpo abaixo (a partir de "##")
no editor. Não inclua o frontmatter na publicação.
-->

## TL;DR

Criei o **NTK (Neural Token Killer)**, um proxy local em Rust que fica entre o Claude Code (e outros editores com hook) e o LLM. Ele intercepta a saída de `Bash`/`cargo test`/`docker logs`/`tsc` e comprime antes de virar contexto. Números medidos: até **92 %** em logs repetitivos do Docker, **56–83 %** em stack traces de várias linguagens, **< 20 ms** de overhead nas camadas regex + tokenizer.

O projeto é **open-source (MIT)** e está bem no início. Preciso de gente para testar, adicionar fixtures de linguagens que ainda não cobrimos, traduzir a documentação, portar o hook para outros editores e fazer benchmarks em GPUs que eu não tenho.

- Repositório: <https://github.com/VALRAW-ALL/ntk>
- Como contribuir: <https://github.com/VALRAW-ALL/ntk/blob/master/CONTRIBUTING.md>
- Como abrir uma issue: <https://github.com/VALRAW-ALL/ntk/blob/master/HOW_TO_OPEN_AN_ISSUE.md>

---

## O problema

Se você usa Claude Code, Cursor, OpenCode ou qualquer outro editor com agente LLM, já percebeu que cada vez que o modelo roda um comando `Bash`, **toda a saída** é empurrada de volta para o contexto do modelo na próxima turn. Um `cargo test` com 200 testes passando, um `docker logs` de 10 mil linhas, um `tsc` com warnings repetidos — tudo isso consome milhares de tokens que, do ponto de vista do modelo, são ruído.

Consequências práticas:

1. **Cap de contexto atingido mais rápido** → a janela acaba no meio de uma sessão produtiva.
2. **Latência de resposta maior** → o modelo precisa processar mais tokens de entrada.
3. **Custo maior** → se você está na API paga.
4. **Perda de foco do modelo** → output verboso no contexto tende a diluir a informação relevante (os erros, os diffs, os avisos).

A solução que já existia — RTK, filtros por regra no shell — funciona bem para casos simples mas é **síncrona** e **cega a semântica**: ela filtra o que o autor da regra já sabia que era ruído, não o que *o modelo* ia considerar ruído.

## A ideia do NTK

Uma pipeline em 4 camadas que roda assincronamente via `PostToolUse` hook:

```
Saída do Bash
  → Hook PostToolUse
    → HTTP POST /compress no daemon local
      → L1 Fast Filter (regex, < 1 ms): ANSI, dedup por template, filtro de stack trace
      → L2 Tokenizer (cl100k_base, < 5 ms): encurtamento de paths, BPE, normalização
      → L3 Local Inference (Phi-3 Mini via Ollama/Candle/llama.cpp, só quando > 300 tokens)
      → L4 Context Injection: prefixa a intenção atual do usuário no prompt da L3
  → Contexto do modelo
```

A L1 e L2 são sempre ligadas e adicionam latência desprezível. A L3 só dispara quando o pós-L1+L2 ainda está acima de 300 tokens — isso evita o overhead de 300-800 ms de inferência para outputs pequenos tipo `git status`. A L4 lê o transcript da sessão do Claude Code para saber qual é a pergunta atual do usuário e prefixa no prompt de compressão, o que dá um ganho mensurável em fixtures onde a camada neural é acionada.

## Números reais

Todos os números abaixo vêm do arquivo `bench/microbench.csv` rodado com `bench/run_all.ps1` contra 15 fixtures deterministicos em `bench/fixtures/`. Nada de estimativa.

| Fixture | Cenário | Economia L1+L2 |
|---|---|---:|
| `docker_logs_repetitive`   | Logs com timestamps repetidos | **92 %** |
| `node_express_trace`       | Stack trace Node.js com node_modules/express | 83 % |
| `cargo_test_failures`      | `cargo test` com 1 falha em 50 | 68 % |
| `python_django_trace`      | Stack trace Django + gunicorn/asgiref | 62 % |
| `stack_trace_java`         | Spring/Tomcat/CGLIB | 60 % |
| `go_panic_trace`           | Go panic + goroutine dumps | 56 % |
| `php_symfony_trace`        | Symfony/HttpKernel + /vendor | 33 % |

A L3 (inferência neural) empurra esses números ainda mais para cima em outputs desestruturados, mas o tempo CPU do Phi-3 Mini é longo (~60 s) sem GPU, então na prática você só liga ela com aceleração CUDA/Metal.

## Por que Rust

- Latência determinística — cada chamada de `Bash` pode ser interceptada; o overhead precisa ser previsível.
- Binário único, zero dependência de runtime — se seu shell abre, NTK roda.
- Multiplataforma sem `#ifdef` — Windows, macOS, Linux compilam a mesma base.
- Tipo forte e enum de backend de inferência — trocar Ollama por Candle ou llama.cpp é um `match` em um lugar.

## O que precisa de ajuda (é aqui que você entra)

Começei o projeto sozinho. A lista de coisas que eu **não tenho como fazer bem** é maior do que a que eu consigo:

1. **Fixtures de novas linguagens** — cada fixture é um par `.txt` + `.meta.json`. Ainda abertos: Elixir/Phoenix, Scala/Akka, Swift/iOS, Flutter/Dart, Clojure, Erlang/OTP. Qualquer log real serve como ponto de partida.
2. **Portar o hook para outros editores** — Claude Code e OpenCode já funcionam. Cursor, Aider, Zed, Continue, Windsurf têm esquemas JSON parecidos. É um script shell autocontido por editor.
3. **Benchmarks em hardware que eu não tenho** — os números de AMD (via Vulkan), Apple Silicon (Metal) e Intel AMX no README são parcialmente estimados. Se você tem uma dessas, rodar `ntk model bench` e mandar um PR com o CSV já resolve.
4. **Traduções** — o site (`docs/`) tem blocos pt-BR e en-US. ES/FR/DE/JA é só copiar e traduzir, zero código.
5. **Quebrar invariantes** — os `cargo test --test compression_invariants` têm 8 invariantes (erro preservado, idempotência, etc). Se você achar uma entrada que viole qualquer um, esse é um dos bugs mais valiosos que tem.

Os arquivos `CONTRIBUTING.md` e `HOW_TO_OPEN_AN_ISSUE.md` estão no repo com tarefas pré-delimitadas. A maior parte cabe em um único PR em menos de uma hora.

## Trade-offs honestos

- **Não é um substituto de LLM remoto** — a L3 usa Phi-3 Mini (3.8B). É bom para sumarizar outputs estruturados, mas não escreve código no seu lugar.
- **Privacidade** — nenhum dado sai da sua máquina, inclusive a telemetria é opt-out e não envia arquivos/paths/conteúdos.
- **Ainda não comparei combinado com RTK** — uma tarefa aberta. Hoje o painel "NTK+RTK combined" no site mostra `N/A` porque não tenho medição real.

## Perguntas abertas (e é aqui que eu quero a sua opinião)

1. Você usa Claude Code / agente de LLM que roda comandos locais? Qual comando mais entope seu contexto?
2. Se um projeto desse tipo existisse antes e você soubesse, teria instalado? O que faria você **não** instalar?
3. Alguma linguagem / framework que você gostaria de ver suportado primeiro?

Qualquer feedback (inclusive "isso é uma má ideia porque X") é mais valioso do que estrela silenciosa.

Obrigado pela leitura. Se chegou até aqui e quer contribuir, o ponto de partida mais curto é <https://github.com/VALRAW-ALL/ntk/blob/master/CONTRIBUTING.md>.
