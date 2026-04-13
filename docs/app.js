// ============================================================
// NTK GitHub Pages - app.js
// i18n (pt-BR + en-US), navigation, animations, utilities
// ============================================================

'use strict';

// ── i18n strings ─────────────────────────────────────────────
const TRANSLATIONS = {
  'en': {
    // Nav
    'nav.home':       'Home',
    'nav.getstart':   'Get Started',
    'nav.commands':   'Commands',
    'nav.metrics':    'Metrics',
    'nav.getstarted': 'Get Started',

    // Hero
    'hero.badge':         'Open Source · Rust · Local AI',
    'hero.title1':        'Neural',
    'hero.title2':        'Token Killer',
    'hero.subtitle':      'Semantic compression proxy daemon for Claude Code. Reduces tool output by <strong>70–99%</strong> before it reaches the LLM context. Uses three progressive compression layers with optional local neural inference.',
    'hero.cta_primary':   '⚡ Get Started',
    'hero.cta_github':    '☆ GitHub',
    'hero.install_label': 'install',
    'hero.stat1':         '% max savings',
    'hero.stat2_val':     '<5ms',
    'hero.stat2':         'L1+L2 latency',
    'hero.stat3':         'compression layers',
    'hero.stat4_val':     '0 deps',
    'hero.stat4':         'runtime required',

    // How it works
    'how.badge':    'How it works',
    'how.title':    'Three layers, one result',
    'how.desc':     'Each layer activates only when needed, keeping latency near zero for small outputs.',
    'how.l1_title': 'Fast Filter',
    'how.l1_desc':  'ANSI removal, line deduplication, test failure extraction. Always on. <1ms.',
    'how.l2_title': 'Tokenizer-Aware',
    'how.l2_desc':  'cl100k_base BPE token counting, path shortening. Always on. <5ms.',
    'how.l3_title': 'Local Inference',
    'how.l3_desc':  'Ollama/Phi-3 Mini with type-specific prompts. Only triggers when output >300 tokens.',

    // Demo
    'demo.badge': 'Live demo',
    'demo.title': 'Installs in 30 seconds',
    'demo.desc':  "One command installs the binary, patches Claude Code's <code style=\"font-family:var(--font-mono);color:var(--color-primaria-clara)\">settings.json</code> with the PostToolUse hook, and creates the config file.",
    'demo.cta':   'View full guide →',

    // Features
    'features.badge': 'Features',
    'features.title': 'Built for developer workflows',
    'feat.fast_title':  'Sub-millisecond L1+L2',
    'feat.fast_desc':   'Regex + tokenizer layers add <5ms overhead to every Bash tool call.',
    'feat.ai_title':    'Local AI, zero cloud',
    'feat.ai_desc':     'Phi-3 Mini runs 100% on your machine. No API keys, no data sent to the cloud.',
    'feat.rtk_title':   'RTK compatible',
    'feat.rtk_desc':    'Works alongside RTK. RTK filters first, NTK semantically summarizes the result.',
    'feat.type_title':  'Type-aware compression',
    'feat.type_desc':   'Different prompts for test output, build errors, logs, diffs. Extracts exactly what matters.',
    'feat.gpu_title':   'GPU acceleration',
    'feat.gpu_desc':    'Auto-detects CUDA, Metal, AMX, AVX-512. L3 latency drops from 800ms to <100ms on GPU.',
    'feat.dash_title':  'Live TUI dashboard + attach mode',
    'feat.dash_desc':   '<code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">ntk start</code> opens a full-screen dashboard with real-time metrics, per-layer stats, and a recent commands console. If the daemon is already running, <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">ntk start</code> attaches to the live TUI without restarting. <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">ntk dashboard</code> prints a static snapshot to stdout and exits. Safe for scripts and CI.',
    'feat.cross_title': 'Cross-platform',
    'feat.cross_desc':  'Single Rust binary. Works on Windows, macOS, and Linux without any runtime dependencies.',

    // Get Started
    'gs.badge':    'Installation guide',
    'gs.title':    'Get Started with NTK',
    'gs.intro':    'NTK is a single Rust binary. The setup takes under 5 minutes.',
    'gs.toc1': '1. Prerequisites',
    'gs.toc2': '2. Install NTK',
    'gs.toc3': '3. Initialize hook',
    'gs.toc4': '4. Install model',
    'gs.toc5': '5. Start daemon',
    'gs.toc6': '6. Verify',
    'gs.toc7': 'Configuration',
    'gs.toc8': 'Uninstall',
    'gs.step1_title': 'Prerequisites',
    'gs.step1_desc':  'NTK requires <strong>Claude Code</strong> (the CLI) installed and configured. Layer 3 inference requires <strong>Ollama</strong> (recommended) or a compatible backend. CPU-only mode works without any AI backend.',
    'gs.step2_title': 'Install NTK',
    'gs.step2_desc':  'The install script downloads the latest release binary for your OS and architecture:',
    'gs.step2_alt':   'Or install from source:',
    'gs.step3_title': 'Initialize hook',
    'gs.step3_desc':  'This patches <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">~/.claude/settings.json</code> to add the PostToolUse hook, copies the hook script, and creates <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">~/.ntk/config.json</code>:',
    'gs.step3_note':  'The <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">-g</code> flag patches the global settings. Use without it for per-project setup. The operation is idempotent. Safe to run multiple times.',
    'gs.step4_title': 'Install model',
    'gs.step4_desc':  'Pull Phi-3 Mini (~2GB) via Ollama for Layer 3 semantic compression:',
    'gs.step4_skip':  'Skip this step to run in L1+L2 only mode (no neural inference, <5ms latency).',
    'gs.step5_title': 'Start daemon',
    'gs.step6_title': 'Verify installation',
    'gs.step7_title': 'Configuration',
    'gs.step7_desc':  'Edit <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">~/.ntk/config.json</code> or place a <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">.ntk.json</code> in your project root for per-project overrides:',
    'gs.step8_title': 'Uninstall',

    // Commands
    'cmd.badge':     'Reference',
    'cmd.title':     'Command Reference',
    'cmd.intro':     'All NTK commands. Prefix every command with <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">ntk</code>.',
    'cmd.search':    'Search commands…',
    'cmd.g_daemon':  'Daemon',
    'cmd.g_setup':   'Setup & Init',
    'cmd.g_model':   'Model',
    'cmd.g_compress':'Compression',
    'cmd.g_metrics': 'Metrics & Analytics',
    'cmd.start':          'Start the compression daemon on port 8765, opening the live TUI dashboard. If daemon is already running, attaches to the live TUI without restarting.',
    'cmd.start_gpu':      'Start with GPU acceleration (CUDA/Metal auto-detected).',
    'cmd.stop':           'Stop the daemon.',
    'cmd.status':         'Show daemon status, loaded model, GPU info, and uptime.',
    'cmd.dashboard':      'Combined static snapshot: daemon status + session gain + ASCII bar chart. Prints to stdout and exits immediately. Safe for scripts and CI.',
    'cmd.init_g':         'Initialize globally: patch settings.json, create ~/.ntk/config.json.',
    'cmd.init_show':      'Display current hook installation status.',
    'cmd.init_uninstall': 'Remove the PostToolUse hook from settings.json.',
    'cmd.init_auto':      'Non-interactive mode for CI/CD pipelines.',
    'cmd.init_hook_only': 'Install hook script only, skipping config.json creation and model setup wizard.',
    'cmd.model_pull':     'Download phi3:mini (default Q5_K_M, ~2GB) via Ollama.',
    'cmd.model_quant':    'Download a specific quantization (q4_k_m, q5_k_m, q6_k).',
    'cmd.model_setup':    'Interactive backend selector (Ollama / Candle / llama.cpp).',
    'cmd.model_test':     'Test model latency and output quality with a sample prompt.',
    'cmd.model_test_debug': 'Verbose test: hardware config, thread counts, timing breakdown, and performance analysis with CPU-tier-aware targets.',
    'cmd.model_bench':    'Benchmark CPU vs GPU inference latency.',
    'cmd.model_list':     'List available models in the configured backend.',
    'cmd.test_compress':  'Run the full pipeline on a captured output file and print result.',
    'cmd.test':           'Run correctness tests on all compression layers. No daemon required.',
    'cmd.test_l3':        'Include Layer 3 inference in the test run.',
    'cmd.bench':          'Benchmark all compression layers (default: 5 runs per payload).',
    'cmd.bench_runs':     'Set number of benchmark runs per payload for stable measurements.',
    'cmd.config':         'Show the active merged configuration (global + project overrides).',
    'cmd.config_file':    'Show configuration from a specific file path.',
    'cmd.metrics':        'Session metrics table in stdout (plain text).',
    'cmd.graph':          'ASCII bar chart + sparkline of savings over time.',
    'cmd.gain':           'Token savings summary (RTK-compatible output format).',
    'cmd.history':        'Last 50 compressed commands with token counts and layer used.',
    'cmd.discover':       'Analyze Claude Code session for missed compression opportunities.',
    'cmd.rtk_note':       '💡 RTK users: prefix with <code style="font-family:var(--font-mono)">rtk ntk &lt;cmd&gt;</code> to also compress NTK\'s own output.',

    // Metrics
    'met.badge':    'Token savings',
    'met.title':    'Savings by Command Type',
    'met.intro':    'Measured against real-world command output captured during development sessions.',
    'met.kpi1':     'Max savings (vitest)',
    'met.kpi1_sub': 'L1+L2+L3',
    'met.kpi2':     'Avg NTK+RTK combined',
    'met.kpi2_sub': 'across all commands',
    'met.kpi3':     'L1+L2 overhead',
    'met.kpi3_sub': 'always on, zero impact',
    'met.kpi4':     'Faster responses',
    'met.kpi4_sub': 'less context = less latency',
    'met.tab_ntk':      'NTK only',
    'met.tab_combined': 'NTK + RTK combined',
    'met.th_category':  'Category',
    'met.th_commands':  'Commands',
    'met.th_savings':   'NTK Savings',
    'met.th_visual':    'Visual',
    'met.th_rtk':       'RTK alone',
    'met.th_ntk_inc':   'NTK incremental',
    'met.th_combined':  'NTK+RTK combined',
    'met.cat_tests':    'Tests',
    'met.cat_build':    'Build',
    'met.cat_git':      'Git',
    'met.cat_gh':       'GitHub CLI',
    'met.cat_pkg':      'Package Managers',
    'met.cat_files':    'Files / Search',
    'met.cat_infra':    'Infrastructure',
    'met.cat_net':      'Network',
    'met.combined_note': 'Combined savings formula: <code style="font-family:var(--font-mono)">1 − (1 − rtk%) × (1 − ntk_incremental%)</code>',
    'met.how_title': 'How savings are measured',
    'met.how_desc':  'Token counts use <strong>cl100k_base</strong> (tiktoken-rs), the same tokenizer as Claude/GPT. Measurements are taken on real captured outputs from development sessions: cargo test suites, tsc compiler errors, vitest runs, git operations, and docker logs. Layer 3 activates only when post-L1+L2 output exceeds 300 tokens, so small outputs incur zero neural inference latency.',

    // GPU latency table
    'met.gpu_badge':      'GPU Acceleration',
    'met.gpu_title':      'Layer 3 Latency by Hardware',
    'met.gpu_intro':      'Phi-3 Mini Q5_K_M (3.8B). GPU drops p95 latency from ~900ms to under 100ms.',
    'met.gpu_th_hw':      'Hardware',
    'met.gpu_th_backend': 'Backend',
    'met.gpu_th_p50':     'p50 latency',
    'met.gpu_th_p95':     'p95 latency',
    'met.gpu_th_notes':   'Notes',
    'met.gpu_note_5060ti':'Ada Lovelace, full offload',
    'met.gpu_note_3060':  '12GB VRAM, full offload',
    'met.gpu_note_m2':    'Unified memory, via Ollama Metal',
    'met.gpu_note_xeon':  'Sapphire Rapids, AMX tiles',
    'met.gpu_note_i7':    '12-core desktop, AVX2',
    'met.gpu_note_i5':    '4-core laptop, baseline CPU',
    'met.gpu_footnote':   'L3 only activates when output exceeds 300 tokens post-L1+L2. Small outputs always use L1+L2 (<5ms).',

    // Footer
    'footer.desc':     'Neural Token Killer: semantic compression proxy for Claude Code. Built with Rust.',
    'footer.docs':     'Docs',
    'footer.project':  'Project',
    'footer.ecosystem':'Ecosystem',
    'footer.install':  'Installation',
    'footer.commands': 'Commands',
    'footer.metrics':  'Metrics',
    'footer.releases': 'Releases',
    'footer.issues':   'Issues',
    'footer.copy':     '© 2025 VALRAW. MIT License.',
    'footer.made':     'Made with',
    'footer.rust':     'in Rust',
  },

  'pt': {
    // Nav
    'nav.home':       'Início',
    'nav.getstart':   'Começar',
    'nav.commands':   'Comandos',
    'nav.metrics':    'Métricas',
    'nav.getstarted': 'Começar',

    // Hero
    'hero.badge':         'Código Aberto · Rust · IA Local',
    'hero.title1':        'Neural',
    'hero.title2':        'Token Killer',
    'hero.subtitle':      'Proxy de compressão semântica para o Claude Code. Reduz a saída das ferramentas em <strong>70–99%</strong> antes de chegar ao contexto do LLM. Usa três camadas progressivas com inferência neural local opcional.',
    'hero.cta_primary':   '⚡ Começar',
    'hero.cta_github':    '☆ GitHub',
    'hero.install_label': 'instalar',
    'hero.stat1':         '% economia máxima',
    'hero.stat2_val':     '<5ms',
    'hero.stat2':         'latência L1+L2',
    'hero.stat3':         'camadas de compressão',
    'hero.stat4_val':     '0 deps',
    'hero.stat4':         'runtime necessário',

    // How it works
    'how.badge':    'Como funciona',
    'how.title':    'Três camadas, um resultado',
    'how.desc':     'Cada camada ativa apenas quando necessário, mantendo a latência próxima de zero para saídas pequenas.',
    'how.l1_title': 'Filtro Rápido',
    'how.l1_desc':  'Remoção de ANSI, deduplicação de linhas, extração de falhas de teste. Sempre ativo. <1ms.',
    'how.l2_title': 'Ciente de Tokens',
    'how.l2_desc':  'Contagem de tokens cl100k_base BPE, encurtamento de caminhos. Sempre ativo. <5ms.',
    'how.l3_title': 'Inferência Local',
    'how.l3_desc':  'Ollama/Phi-3 Mini com prompts específicos por tipo. Só ativa quando saída >300 tokens.',

    // Demo
    'demo.badge': 'Demo ao vivo',
    'demo.title': 'Instala em 30 segundos',
    'demo.desc':  'Um comando instala o binário, injeta o hook PostToolUse no <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">settings.json</code> do Claude Code e cria o arquivo de configuração.',
    'demo.cta':   'Ver guia completo →',

    // Features
    'features.badge': 'Recursos',
    'features.title': 'Criado para fluxos de trabalho de desenvolvimento',
    'feat.fast_title':  'L1+L2 sub-milissegundo',
    'feat.fast_desc':   'Camadas de regex + tokenizer adicionam <5ms de overhead a cada chamada de ferramenta Bash.',
    'feat.ai_title':    'IA local, zero nuvem',
    'feat.ai_desc':     'Phi-3 Mini roda 100% na sua máquina. Sem chaves de API, sem dados enviados à nuvem.',
    'feat.rtk_title':   'Compatível com RTK',
    'feat.rtk_desc':    'Funciona junto com o RTK. O RTK filtra primeiro, o NTK resume semanticamente o resultado.',
    'feat.type_title':  'Compressão ciente do tipo',
    'feat.type_desc':   'Prompts diferentes para saída de testes, erros de build, logs, diffs. Extrai exatamente o que importa.',
    'feat.gpu_title':   'Aceleração GPU',
    'feat.gpu_desc':    'Detecta automaticamente CUDA, Metal, AMX, AVX-512. Latência L3 cai de 800ms para <100ms em GPU.',
    'feat.dash_title':  'Dashboard TUI ao vivo + modo attach',
    'feat.dash_desc':   '<code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">ntk start</code> abre um dashboard em tela cheia com métricas em tempo real, estatísticas por camada e console dos últimos comandos. Se o daemon já estiver rodando, <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">ntk start</code> reconecta ao TUI sem reiniciar. <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">ntk dashboard</code> imprime um snapshot estático no stdout e sai. Seguro para scripts e CI.',
    'feat.cross_title': 'Multiplataforma',
    'feat.cross_desc':  'Binário único em Rust. Funciona no Windows, macOS e Linux sem nenhuma dependência de runtime.',

    // Get Started
    'gs.badge':    'Guia de instalação',
    'gs.title':    'Comece com o NTK',
    'gs.intro':    'O NTK é um binário único em Rust. A configuração leva menos de 5 minutos.',
    'gs.toc1': '1. Pré-requisitos',
    'gs.toc2': '2. Instalar NTK',
    'gs.toc3': '3. Inicializar hook',
    'gs.toc4': '4. Instalar modelo',
    'gs.toc5': '5. Iniciar daemon',
    'gs.toc6': '6. Verificar',
    'gs.toc7': 'Configuração',
    'gs.toc8': 'Desinstalar',
    'gs.step1_title': 'Pré-requisitos',
    'gs.step1_desc':  'O NTK requer o <strong>Claude Code</strong> (CLI) instalado e configurado. A inferência da Camada 3 requer o <strong>Ollama</strong> (recomendado) ou backend compatível. O modo somente-CPU funciona sem nenhum backend de IA.',
    'gs.step2_title': 'Instalar NTK',
    'gs.step2_desc':  'O script de instalação baixa o binário da versão mais recente para seu OS e arquitetura:',
    'gs.step2_alt':   'Ou instale a partir do código-fonte:',
    'gs.step3_title': 'Inicializar hook',
    'gs.step3_desc':  'Isso injeta o hook PostToolUse no <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">~/.claude/settings.json</code>, copia o script do hook e cria o <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">~/.ntk/config.json</code>:',
    'gs.step3_note':  'A flag <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">-g</code> injeta nas configurações globais. Use sem ela para configuração por projeto. A operação é idempotente. Seguro executar múltiplas vezes.',
    'gs.step4_title': 'Instalar modelo',
    'gs.step4_desc':  'Baixe o Phi-3 Mini (~2GB) via Ollama para compressão semântica da Camada 3:',
    'gs.step4_skip':  'Pule esta etapa para rodar no modo somente L1+L2 (sem inferência neural, latência <5ms).',
    'gs.step5_title': 'Iniciar daemon',
    'gs.step6_title': 'Verificar instalação',
    'gs.step7_title': 'Configuração',
    'gs.step7_desc':  'Edite o <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">~/.ntk/config.json</code> ou coloque um <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">.ntk.json</code> na raiz do projeto para substituições por projeto:',
    'gs.step8_title': 'Desinstalar',

    // Commands
    'cmd.badge':     'Referência',
    'cmd.title':     'Referência de Comandos',
    'cmd.intro':     'Todos os comandos NTK. Prefixe cada comando com <code style="font-family:var(--font-mono);color:var(--color-primaria-clara)">ntk</code>.',
    'cmd.search':    'Buscar comandos…',
    'cmd.g_daemon':  'Daemon',
    'cmd.g_setup':   'Setup e Init',
    'cmd.g_model':   'Modelo',
    'cmd.g_compress':'Compressão',
    'cmd.g_metrics': 'Métricas e Análises',
    'cmd.start':          'Inicia o daemon de compressão na porta 8765 abrindo o dashboard TUI ao vivo. Se o daemon já estiver rodando, reconecta ao TUI sem reiniciar.',
    'cmd.start_gpu':      'Inicia com aceleração GPU (CUDA/Metal detectado automaticamente).',
    'cmd.stop':           'Para o daemon.',
    'cmd.status':         'Exibe status do daemon, modelo carregado, info GPU e uptime.',
    'cmd.dashboard':      'Snapshot estático combinado: status do daemon + economia da sessão + gráfico de barras ASCII. Imprime no stdout e sai. Seguro para scripts e CI.',
    'cmd.init_g':         'Inicializa globalmente: injeta settings.json, cria ~/.ntk/config.json.',
    'cmd.init_show':      'Exibe o status atual de instalação do hook.',
    'cmd.init_uninstall': 'Remove o hook PostToolUse do settings.json.',
    'cmd.init_auto':      'Modo não-interativo para pipelines de CI/CD.',
    'cmd.init_hook_only': 'Instala apenas o script do hook, ignorando criação do config.json e wizard de modelo.',
    'cmd.model_pull':     'Baixa phi3:mini (padrão Q5_K_M, ~2GB) via Ollama.',
    'cmd.model_quant':    'Baixa uma quantização específica (q4_k_m, q5_k_m, q6_k).',
    'cmd.model_setup':    'Seletor interativo de backend (Ollama / Candle / llama.cpp).',
    'cmd.model_test':     'Testa latência e qualidade do modelo com um prompt de amostra.',
    'cmd.model_test_debug': 'Teste verbose: config de hardware, threads, breakdown de tempo e análise por nível de CPU.',
    'cmd.model_bench':    'Benchmarks de latência CPU vs GPU.',
    'cmd.model_list':     'Lista os modelos disponíveis no backend configurado.',
    'cmd.test_compress':  'Roda o pipeline completo em um arquivo de saída capturado e exibe o resultado.',
    'cmd.test':           'Executa testes de correção em todas as camadas de compressão. Não requer daemon.',
    'cmd.test_l3':        'Inclui inferência da Camada 3 no teste.',
    'cmd.bench':          'Benchmarks de todas as camadas de compressão (padrão: 5 execuções por payload).',
    'cmd.bench_runs':     'Define o número de execuções por payload para medições mais estáveis.',
    'cmd.config':         'Exibe a configuração ativa mesclada (global + substituições do projeto).',
    'cmd.config_file':    'Exibe a configuração de um arquivo específico.',
    'cmd.metrics':        'Tabela de métricas da sessão no stdout (texto simples).',
    'cmd.graph':          'Gráfico de barras ASCII + sparkline de economia ao longo do tempo.',
    'cmd.gain':           'Resumo de economia de tokens (formato compatível com RTK).',
    'cmd.history':        'Últimos 50 comandos comprimidos com contagem de tokens e camada utilizada.',
    'cmd.discover':       'Analisa a sessão do Claude Code em busca de oportunidades de compressão perdidas.',
    'cmd.rtk_note':       '💡 Usuários RTK: prefixe com <code style="font-family:var(--font-mono)">rtk ntk &lt;cmd&gt;</code> para também comprimir a saída do próprio NTK.',

    // Metrics
    'met.badge':    'Economia de tokens',
    'met.title':    'Economia por Tipo de Comando',
    'met.intro':    'Medido com saídas de comandos reais capturadas durante sessões de desenvolvimento.',
    'met.kpi1':     'Economia máxima (vitest)',
    'met.kpi1_sub': 'L1+L2+L3',
    'met.kpi2':     'Média NTK+RTK combinados',
    'met.kpi2_sub': 'em todos os comandos',
    'met.kpi3':     'Overhead L1+L2',
    'met.kpi3_sub': 'sempre ativo, impacto zero',
    'met.kpi4':     'Respostas mais rápidas',
    'met.kpi4_sub': 'menos contexto = menos latência',
    'met.tab_ntk':      'Somente NTK',
    'met.tab_combined': 'NTK + RTK combinados',
    'met.th_category':  'Categoria',
    'met.th_commands':  'Comandos',
    'met.th_savings':   'Economia NTK',
    'met.th_visual':    'Visual',
    'met.th_rtk':       'RTK isolado',
    'met.th_ntk_inc':   'NTK incremental',
    'met.th_combined':  'NTK+RTK combinados',
    'met.cat_tests':    'Testes',
    'met.cat_build':    'Build',
    'met.cat_git':      'Git',
    'met.cat_gh':       'GitHub CLI',
    'met.cat_pkg':      'Gerenciadores de Pacotes',
    'met.cat_files':    'Arquivos / Busca',
    'met.cat_infra':    'Infraestrutura',
    'met.cat_net':      'Rede',
    'met.combined_note': 'Fórmula de economia combinada: <code style="font-family:var(--font-mono)">1 − (1 − rtk%) × (1 − ntk_incremental%)</code>',
    'met.how_title': 'Como a economia é medida',
    'met.how_desc':  'Contagens de tokens usam <strong>cl100k_base</strong> (tiktoken-rs), o mesmo tokenizador do Claude/GPT. As medições são feitas em saídas reais capturadas durante sessões de desenvolvimento: suítes cargo test, erros tsc, execuções vitest, operações git e logs docker. A Camada 3 só ativa quando a saída pós-L1+L2 supera 300 tokens, então saídas pequenas não incorrem em nenhuma latência de inferência neural.',

    // Tabela de latência GPU
    'met.gpu_badge':      'Aceleração GPU',
    'met.gpu_title':      'Latência da Camada 3 por Hardware',
    'met.gpu_intro':      'Phi-3 Mini Q5_K_M (3.8B). GPU reduz a latência p95 de ~900ms para menos de 100ms.',
    'met.gpu_th_hw':      'Hardware',
    'met.gpu_th_backend': 'Backend',
    'met.gpu_th_p50':     'Latência p50',
    'met.gpu_th_p95':     'Latência p95',
    'met.gpu_th_notes':   'Notas',
    'met.gpu_note_5060ti':'Ada Lovelace, offload completo',
    'met.gpu_note_3060':  '12GB VRAM, offload completo',
    'met.gpu_note_m2':    'Memória unificada, via Ollama Metal',
    'met.gpu_note_xeon':  'Sapphire Rapids, tiles AMX',
    'met.gpu_note_i7':    'Desktop 12-core, AVX2',
    'met.gpu_note_i5':    'Laptop 4-core, CPU base',
    'met.gpu_footnote':   'L3 só ativa quando a saída supera 300 tokens pós-L1+L2. Saídas pequenas sempre usam L1+L2 (<5ms).',

    // Footer
    'footer.desc':     'Neural Token Killer: proxy de compressão semântica para o Claude Code. Feito em Rust.',
    'footer.docs':     'Docs',
    'footer.project':  'Projeto',
    'footer.ecosystem':'Ecossistema',
    'footer.install':  'Instalação',
    'footer.commands': 'Comandos',
    'footer.metrics':  'Métricas',
    'footer.releases': 'Versões',
    'footer.issues':   'Issues',
    'footer.copy':     '© 2025 VALRAW. Licença MIT.',
    'footer.made':     'Feito com',
    'footer.rust':     'em Rust',
  }
};

// ── State ─────────────────────────────────────────────────────
let currentLang = 'en';
let currentPage = 'home';

// ── Language detection ────────────────────────────────────────
function detectLang() {
  const saved = localStorage.getItem('ntk-lang');
  if (saved) return saved;
  const nav = navigator.language || navigator.userLanguage || 'en';
  return nav.toLowerCase().startsWith('pt') ? 'pt' : 'en';
}

// ── Apply translations ────────────────────────────────────────
function applyLang(lang) {
  currentLang = lang;
  localStorage.setItem('ntk-lang', lang);
  const dict = TRANSLATIONS[lang] || TRANSLATIONS['en'];

  // data-i18n attributes
  document.querySelectorAll('[data-i18n]').forEach(el => {
    const key = el.getAttribute('data-i18n');
    if (dict[key] !== undefined) {
      el.innerHTML = dict[key];
    }
  });

  // data-i18n-placeholder attributes
  document.querySelectorAll('[data-i18n-placeholder]').forEach(el => {
    const key = el.getAttribute('data-i18n-placeholder');
    if (dict[key] !== undefined) {
      el.placeholder = dict[key];
    }
  });

  // lang toggle button label
  const btn = document.getElementById('langToggle');
  if (btn) btn.textContent = lang === 'pt' ? 'PT' : 'EN';

  // html lang attribute
  document.documentElement.lang = lang === 'pt' ? 'pt-BR' : 'en';
}

function toggleLang() {
  applyLang(currentLang === 'en' ? 'pt' : 'en');
}

// ── Page navigation ───────────────────────────────────────────
function navigateTo(page) {
  // Hide all pages
  document.querySelectorAll('.page').forEach(p => p.classList.remove('active'));
  // Show target
  const el = document.getElementById('page-' + page);
  if (el) el.classList.add('active');

  // Update nav links
  document.querySelectorAll('.nav-link[data-page]').forEach(btn => {
    btn.classList.toggle('active', btn.getAttribute('data-page') === page);
  });

  currentPage = page;
  window.scrollTo({ top: 0, behavior: 'smooth' });

  // Trigger progress bar animation when navigating to metrics
  if (page === 'metrics') {
    setTimeout(animateProgressBars, 200);
  }
}

// ── Mobile menu ───────────────────────────────────────────────
function toggleMobileMenu() {
  const nav = document.getElementById('mobileNav');
  nav.classList.toggle('open');
}

// ── Header scroll effect ──────────────────────────────────────
function initHeaderScroll() {
  const header = document.getElementById('header');
  window.addEventListener('scroll', () => {
    header.classList.toggle('scrolled', window.scrollY > 40);
  }, { passive: true });
}

// ── Copy to clipboard ─────────────────────────────────────────
function copyCode(btn) {
  const pre = btn.closest('.code-block').querySelector('pre');
  if (!pre) return;
  navigator.clipboard.writeText(pre.innerText).then(() => {
    const orig = btn.innerHTML;
    btn.innerHTML = '✓ copied';
    btn.style.color = 'var(--color-acento-verde)';
    setTimeout(() => { btn.innerHTML = orig; btn.style.color = ''; }, 1500);
  });
}

function copyInstall(el) {
  const code = el.querySelector('code');
  if (!code) return;
  navigator.clipboard.writeText(code.innerText).then(() => {
    const icon = el.querySelector('.copy-icon');
    if (icon) { icon.textContent = '✓'; setTimeout(() => { icon.textContent = '⎘'; }, 1500); }
  });
}

// ── Metrics tab switch ────────────────────────────────────────
function switchMetricsTab(tab) {
  document.querySelectorAll('.metrics-tab').forEach((btn, i) => {
    btn.classList.toggle('active', (tab === 'ntk' && i === 0) || (tab === 'combined' && i === 1));
  });
  document.getElementById('panel-ntk').classList.toggle('active', tab === 'ntk');
  document.getElementById('panel-combined').classList.toggle('active', tab === 'combined');
  setTimeout(animateProgressBars, 100);
}

// ── Progress bar animation ────────────────────────────────────
function animateProgressBars() {
  document.querySelectorAll('.progress-bar-fill').forEach(bar => {
    const target = parseInt(bar.getAttribute('data-width') || '0', 10);
    bar.style.width = target + '%';
  });
}

// ── Counter animation ─────────────────────────────────────────
function animateCounters() {
  document.querySelectorAll('[data-counter]').forEach(el => {
    const target = parseInt(el.getAttribute('data-counter'), 10);
    let current = 0;
    const step = Math.ceil(target / 40);
    const interval = setInterval(() => {
      current = Math.min(current + step, target);
      el.textContent = current + (target >= 10 ? '%' : '');
      if (current >= target) clearInterval(interval);
    }, 30);
  });
}

// ── IntersectionObserver for fade-in ─────────────────────────
function initFadeIn() {
  const observer = new IntersectionObserver(entries => {
    entries.forEach(entry => {
      if (entry.isIntersecting) {
        entry.target.classList.add('visible');
        observer.unobserve(entry.target);
      }
    });
  }, { threshold: 0.1 });

  document.querySelectorAll('.fade-in').forEach(el => observer.observe(el));
}

// ── Command search ────────────────────────────────────────────
function filterCmds(query) {
  const q = query.toLowerCase().trim();
  document.querySelectorAll('.cmd-row').forEach(row => {
    const text = row.textContent.toLowerCase();
    row.style.display = !q || text.includes(q) ? '' : 'none';
  });
  document.querySelectorAll('.cmd-group').forEach(group => {
    const visible = Array.from(group.querySelectorAll('.cmd-row')).some(r => r.style.display !== 'none');
    group.style.display = visible ? '' : 'none';
  });
}

// ── TOC scroll spy (Get Started page) ────────────────────────
function initTocSpy() {
  const toc = document.getElementById('tocList');
  if (!toc) return;
  const links = toc.querySelectorAll('a[href^="#"]');
  const observer = new IntersectionObserver(entries => {
    entries.forEach(entry => {
      if (entry.isIntersecting) {
        links.forEach(l => l.classList.toggle('active', l.getAttribute('href') === '#' + entry.target.id));
      }
    });
  }, { rootMargin: '-20% 0px -70% 0px' });

  links.forEach(l => {
    const target = document.querySelector(l.getAttribute('href'));
    if (target) observer.observe(target);
  });
}

// ── Init ──────────────────────────────────────────────────────
document.addEventListener('DOMContentLoaded', () => {
  applyLang(detectLang());
  initHeaderScroll();
  initFadeIn();
  initTocSpy();

  // Animate hero counters after a short delay
  setTimeout(animateCounters, 400);

  // Animate hero progress if on metrics
  if (currentPage === 'metrics') animateProgressBars();
});

// Re-run fade-in observer on page switch
const origNavigateTo = navigateTo;
