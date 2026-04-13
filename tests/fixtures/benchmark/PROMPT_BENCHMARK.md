# NTK Benchmark Prompt

Execute este prompt **identico** nas duas sessoes para comparar o consumo de tokens:
- Sessao A: Claude Code **sem** o hook NTK ativo
- Sessao B: Claude Code **com** o hook NTK ativo (`ntk start` + hook instalado)

---

## Prompt para executar no Claude Code

Cole exatamente o texto abaixo no prompt do Claude Code (nao modifique nada):

---

Leia os tres arquivos a seguir e para cada um responda: quantas linhas tem, qual o tipo de conteudo (build, logs, testes, etc), e qual o principal problema ou evento descrito.

Arquivo 1: tests/fixtures/benchmark/layer1_ansi_terminal.txt
Arquivo 2: tests/fixtures/benchmark/layer2_code_generation.txt
Arquivo 3: tests/fixtures/benchmark/layer3_verbose_logs.txt

---

## Como medir os tokens consumidos

Apos a resposta do Claude Code, execute no terminal:

```bash
ntk metrics
```

O campo `tokens_in` da ultima sessao corresponde aos tokens enviados ao modelo
(output das ferramentas + historico). Esse e o numero a comparar.

Ou use `ntk gain` para ver o resumo de economia da sessao atual.

---

## Tabela de resultados

Preencha apos cada execucao:

| Medicao                  | Sem NTK | Com NTK | Reducao |
|--------------------------|---------|---------|---------|
| Tokens consumidos (input)|         |         |         |
| Tempo de resposta        |         |         |         |
| Layer acionada           | N/A     | L1/L2/L3|         |
| Qualidade da resposta    | OK/NOK  | OK/NOK  |         |

---

## Arquivos de teste e o que cada layer deve fazer com eles

| Arquivo                      | Tamanho aprox. | Layer esperada | Motivo                                               |
|------------------------------|----------------|----------------|------------------------------------------------------|
| `layer1_ansi_terminal.txt`   | ~22 KB         | L1             | Rico em ANSI, linhas duplicadas — L1 colapsa tudo    |
| `layer2_code_generation.txt` | ~14 KB         | L2             | Erros de compilacao com file paths — L2 encurta paths|
| `layer3_verbose_logs.txt`    | ~16 KB         | L3             | Logs verbosos com repeticao — L3 resume semanticamente|

---

## Comandos uteis

```bash
# Iniciar o daemon antes do teste Com NTK
ntk start

# Verificar que o daemon esta rodando
ntk status

# Ver metricas da sessao apos o teste
ntk metrics

# Ver ganho resumido (formato RTK-compatible)
ntk gain

# Parar o daemon apos os testes
ntk stop
```
