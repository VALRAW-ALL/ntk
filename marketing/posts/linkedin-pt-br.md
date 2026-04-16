---
platform: linkedin
language: pt-BR
ideal_char_count: 1300-2000
hashtags:
  - "#Rust"
  - "#IA"
  - "#LLM"
  - "#OpenSource"
  - "#DevTools"
  - "#ClaudeCode"
  - "#OpenToContribute"
  - "#InteligenciaArtificial"
  - "#DesenvolvimentoDeSoftware"
hook_strategy: "pergunta + número concreto nas 3 primeiras linhas (antes do 'ver mais')"
---

<!--
LinkedIn: cole o corpo abaixo direto no editor. As hashtags vão no
final, já formatadas. Sem imagens inline (LinkedIn prefere 1 imagem
anexada como preview do link, use assets/rtk.webp ou screenshot da
landing ntk.valraw.com).
-->

Seu agente de LLM também roda `cargo test` e depois desperdiça 1500 tokens com "test ok" repetido?

Passei os últimos meses batendo nesse problema e acabei construindo uma solução. Compartilho aqui porque está aberto para contribuição.

---

**O problema**

Editores com agente — Claude Code, Cursor, OpenCode — executam comandos de terminal em loop. Toda saída volta pro contexto do modelo: `docker logs` com 10 mil linhas, `tsc` com warnings repetidos, stack traces de 300 linhas. Isso consome janela de contexto, aumenta latência e, quando você está em API paga, aumenta custo.

**O que construí**

NTK (Neural Token Killer): um daemon em Rust que fica entre o editor e o LLM, interceptando saídas via hook `PostToolUse` e comprimindo antes de virar contexto.

Pipeline de 4 camadas:
→ L1 regex (ANSI, dedup por template, filtro de stack trace em 11 linguagens)
→ L2 tokenizer cl100k_base (encurtamento de paths, normalização de hashes)
→ L3 inferência local com Phi-3 Mini (Ollama / Candle / llama.cpp — opcional)
→ L4 injeção de contexto lendo a intenção do usuário no transcript da sessão

**Números medidos (não estimados):**
— 92 % de economia em Docker logs repetitivos
— 56-83 % em stack traces (Java, Python, Go, Node, PHP, C# e outras)
— < 20 ms de overhead nas duas primeiras camadas
— 100 % open-source, MIT, roda offline

**Por que eu escrevo isso aqui:**

Comecei o projeto sozinho. Para evoluir, precisa de gente que eu não alcanço sozinho — devs com logs de linguagens que ainda não cobrimos (Elixir, Swift, Flutter, Scala), gente com GPU diferente da minha para benchmarks reais, tradutores para a documentação, ports do hook para outros editores.

Estou chamando de "iniciativa aberta" no README de propósito: não é produto, é um esforço colaborativo que começa agora.

**Se você:**
→ É dev e gasta horas com agentes de LLM, comenta aí qual comando mais entope seu contexto
→ Quer contribuir, o repo tem um CONTRIBUTING.md com tarefas pré-delimitadas (maior parte cabe em 1 PR < 1h)
→ Só quer acompanhar, deixa uma estrela que o projeto avança no radar

Link: github.com/VALRAW-ALL/ntk

E a pergunta que me intriga: **compressão semântica de contexto é o próximo "padrão implícito" dos agentes de LLM, ou só uma otimização de nicho?** Gostaria de ouvir quem trabalha com o assunto.

---

#Rust #IA #LLM #OpenSource #DevTools #ClaudeCode #OpenToContribute #InteligenciaArtificial #DesenvolvimentoDeSoftware
