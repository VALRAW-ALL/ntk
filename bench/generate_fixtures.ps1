#!/usr/bin/env pwsh
# Generate deterministic fixture files for NTK microbench.
# Produces bench/fixtures/*.txt + bench/fixtures/*.meta.json.

$ErrorActionPreference = 'Stop'

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$out = Join-Path $here 'fixtures'
New-Item -ItemType Directory -Force -Path $out | Out-Null

function Write-Fixture([string]$name, [string]$body, $meta) {
    $txtPath  = Join-Path $out "$name.txt"
    $metaPath = Join-Path $out "$name.meta.json"
    [System.IO.File]::WriteAllText($txtPath, $body, [System.Text.UTF8Encoding]::new($false))
    $json = $meta | ConvertTo-Json -Depth 5
    [System.IO.File]::WriteAllText($metaPath, $json + [Environment]::NewLine, [System.Text.UTF8Encoding]::new($false))
}

# ------------------------------------------------------------------
# 1. cargo_build_verbose -- L1 dedup of "Compiling X"
# ------------------------------------------------------------------
$crates = @(
    'serde','serde_json','tokio','axum','reqwest','hyper','bytes','http',
    'regex','once_cell','clap','anyhow','thiserror','chrono','uuid','sqlx',
    'tiktoken-rs','candle-core','candle-nn','tracing'
)
$lines = New-Object System.Collections.Generic.List[string]
for ($i = 0; $i -lt $crates.Count; $i++) {
    $c = $crates[$i]
    $ver = '{0}.{1}.{2}' -f (1 + ($i % 3)), ($i % 10), (($i * 7) % 20)
    $lines.Add("   Compiling $c v$ver")
    $lines.Add("   Compiling $($c)_derive v$ver")
}
foreach ($c in $crates[0..4]) {
    $lines.Add("warning: unused variable: ``foo``")
    $lines.Add("  --> /src/$c/lib.rs:42:9")
    $lines.Add("   |")
    $lines.Add("42 |     let foo = 1;")
    $lines.Add("   |         ^^^ help: if this is intentional, prefix it with an underscore: ``_foo``")
    $lines.Add("")
}
$lines.Add('   Compiling ntk v0.2.24 (C:\Users\dev\ntk)')
$lines.Add('    Finished `release` profile [optimized] target(s) in 1m 03s')
$body = ($lines -join "`n") + "`n"
Write-Fixture 'cargo_build_verbose' $body @{
    category='build'; expected_layer=2; min_ratio=0.15
    command='cargo build --release --verbose'
    description='Verbose cargo build output with many Compiling lines.'
}

# ------------------------------------------------------------------
# 2. cargo_test_failures -- L1 filters passes, keeps failures
# ------------------------------------------------------------------
$lines = New-Object System.Collections.Generic.List[string]
$lines.Add('running 42 tests')
for ($i = 0; $i -lt 25; $i++) { $lines.Add('test tests::test_case_{0:D2} ... ok' -f $i) }
$lines.Add('test tests::test_connection_timeout ... FAILED')
$lines.Add('test tests::test_auth_header_malformed ... FAILED')
for ($i = 25; $i -lt 42; $i++) { $lines.Add('test tests::test_extra_{0:D2} ... ok' -f $i) }
$lines.Add('')
$lines.Add('failures:')
$lines.Add('')
$lines.Add('---- tests::test_connection_timeout stdout ----')
$lines.Add("thread 'tests::test_connection_timeout' panicked at 'assertion failed: ``(left == right)``")
$lines.Add('  left: `200`,')
$lines.Add(" right: ``408```', tests/unit/api.rs:142:5")
$lines.Add('note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace')
$lines.Add('')
$lines.Add('---- tests::test_auth_header_malformed stdout ----')
$lines.Add("thread 'tests::test_auth_header_malformed' panicked at 'invalid header: bearer token length mismatch'")
$lines.Add('  expected 64 chars, got 32')
$lines.Add('  at tests/unit/auth.rs:88:5')
$lines.Add('')
$lines.Add('failures:')
$lines.Add('    tests::test_auth_header_malformed')
$lines.Add('    tests::test_connection_timeout')
$lines.Add('')
$lines.Add('test result: FAILED. 40 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out')
$body = ($lines -join "`n") + "`n"
Write-Fixture 'cargo_test_failures' $body @{
    category='test'; expected_layer=2; min_ratio=0.30
    command='cargo test'
    description='Cargo test with many passes and a few failures.'
}

# ------------------------------------------------------------------
# 3. tsc_errors_node_modules -- L2 path shortening
# ------------------------------------------------------------------
$paths = @(
    'src/components/layout/header/navigation/MainMenu.tsx',
    'src/components/layout/header/navigation/MobileMenu.tsx',
    'src/components/layout/footer/SiteFooter.tsx',
    'src/features/auth/hooks/useAuthSession.ts',
    'src/features/auth/providers/AuthProvider.tsx',
    'node_modules/@tanstack/react-query/build/modern/useQuery.d.ts',
    'node_modules/react-router-dom/dist/index.d.ts',
    'src/api/endpoints/users/getUserProfile.ts',
    'src/api/endpoints/users/updateUserSettings.ts',
    'src/utils/validators/schemas/userSchema.ts'
)
$messages = @(
    "Type 'string' is not assignable to type 'number'.",
    "Cannot find name 'Request'.",
    "Property 'id' does not exist on type 'User | null'.",
    "Argument of type 'unknown' is not assignable to parameter of type 'string'.",
    'Module ''./types'' has no exported member ''UserRole''.'
)
$lines = New-Object System.Collections.Generic.List[string]
for ($i = 0; $i -lt $paths.Count; $i++) {
    $p = $paths[$i]
    $lineNo = 10 + ($i * 7)
    $col = 3 + ($i % 15)
    $code = 'TS{0}' -f (2300 + (($i * 13) % 200))
    $msg = $messages[$i % 5]
    $lines.Add("$($p):$($lineNo):$($col) - error $($code): $msg")
    $lines.Add('')
    $lines.Add("$lineNo   const result = fetchUser(id);")
    $lines.Add('    ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~')
    $lines.Add('')
}
$lines.Add('Found 10 errors in 10 files.')
$body = ($lines -join "`n") + "`n"
Write-Fixture 'tsc_errors_node_modules' $body @{
    category='typecheck'; expected_layer=2; min_ratio=0.10
    command='npx tsc --noEmit'
    description='TypeScript errors with long src/ and node_modules/ paths.'
}

# ------------------------------------------------------------------
# 4. docker_logs_repetitive -- massive L1 dedup
# ------------------------------------------------------------------
$lines = New-Object System.Collections.Generic.List[string]
for ($i = 0; $i -lt 80; $i++) {
    $lines.Add('2026-04-15T10:23:{0:D2}Z INFO [api] GET /health 200 12ms user=anonymous' -f ($i % 60))
}
for ($i = 0; $i -lt 20; $i++) {
    $lines.Add('2026-04-15T10:24:{0:D2}Z INFO [api] GET /health 200 13ms user=anonymous' -f ($i % 60))
}
$lines.Add('2026-04-15T10:25:01Z ERROR [db] connection pool exhausted, retrying in 500ms')
for ($i = 0; $i -lt 30; $i++) {
    $sec = ($i + 2) % 60
    $n = $i + 1
    $lines.Add("2026-04-15T10:25:$('{0:D2}' -f $sec)Z WARN  [db] retry $n/30 failed: timeout")
}
$lines.Add('2026-04-15T10:26:00Z INFO  [db] connection pool restored')
for ($i = 0; $i -lt 40; $i++) {
    $lines.Add('2026-04-15T10:26:{0:D2}Z INFO [api] GET /health 200 10ms user=anonymous' -f ($i % 60))
}
$body = ($lines -join "`n") + "`n"
Write-Fixture 'docker_logs_repetitive' $body @{
    category='logs'; expected_layer=2; min_ratio=0.50
    command='docker logs api-service --tail 200'
    description='Highly repetitive logs -- L1 dedup should dominate.'
}

# ------------------------------------------------------------------
# 5. generic_long_log -- L3 needed
# ------------------------------------------------------------------
$lines = New-Object System.Collections.Generic.List[string]
for ($i = 0; $i -lt 40; $i++) {
    $lines.Add(
        "[INFO] Processing batch $($i): loaded 1024 records from upstream queue, " +
        "transforming payload with schema v$(3 + ($i % 4)). Validation passed for " +
        "1022 records, 2 rejected due to missing 'email' field. Forwarding to " +
        "downstream service at endpoint https://internal.api.example.com/ingest/v3. " +
        "Average latency 42ms, p99 87ms, zero retries triggered."
    )
}
$body = ($lines -join "`n") + "`n"
Write-Fixture 'generic_long_log' $body @{
    category='logs_unstructured'; expected_layer=3; min_ratio=0.50
    command='tail -n 40 /var/log/ingest.log'
    description='Long unstructured prose log -- L3 summarization expected.'
}

# ------------------------------------------------------------------
# 6. already_short -- hook skips (< 500 chars)
# ------------------------------------------------------------------
Write-Fixture 'already_short' "error: file not found`n" @{
    category='short'; expected_layer=0; min_ratio=0.0
    command='echo error'
    description='Short output below the hook MinChars (500).'
}

# ------------------------------------------------------------------
# 7. git_diff_large -- L2 preserves structure
# ------------------------------------------------------------------
$lines = New-Object System.Collections.Generic.List[string]
$lines.Add('diff --git a/src/server.rs b/src/server.rs')
$lines.Add('index abc1234..def5678 100644')
$lines.Add('--- a/src/server.rs')
$lines.Add('+++ b/src/server.rs')
$lines.Add('@@ -45,12 +45,28 @@ pub struct CompressResponse {')
$lines.Add('     pub compressed: String,')
$lines.Add('     pub ratio: f32,')
$lines.Add('     pub layer: u8,')
$lines.Add('     pub tokens_before: usize,')
$lines.Add('     pub tokens_after: usize,')
$lines.Add('+    pub tokens_after_l1: Option<usize>,')
$lines.Add('+    pub tokens_after_l2: Option<usize>,')
$lines.Add('+    pub tokens_after_l3: Option<usize>,')
$lines.Add('+    pub latency_ms: LayerLatency,')
$lines.Add(' }')
for ($i = 0; $i -lt 10; $i++) {
    $lines.Add('')
    $lines.Add("diff --git a/src/compressor/module_$i.rs b/src/compressor/module_$i.rs")
    $lines.Add(('index {0:D4}abc..{0:D4}def 100644' -f $i))
    $lines.Add("--- a/src/compressor/module_$i.rs")
    $lines.Add("+++ b/src/compressor/module_$i.rs")
    $lines.Add(('@@ -{0},5 +{0},8 @@' -f (1 + ($i * 10))))
    $lines.Add(' pub fn process(input: &str) -> Result<Output> {')
    $lines.Add('     let tokens = count_tokens(input)?;')
    $lines.Add('-    Ok(Output { tokens, text: input.to_string() })')
    $lines.Add('+    let filtered = strip_noise(input);')
    $lines.Add('+    let tokens = count_tokens(&filtered)?;')
    $lines.Add('+    Ok(Output { tokens, text: filtered })')
    $lines.Add(' }')
}
$body = ($lines -join "`n") + "`n"
Write-Fixture 'git_diff_large' $body @{
    category='diff'; expected_layer=2; min_ratio=0.05
    command='git diff HEAD~3'
    description='Git diff with many hunks.'
}

# ------------------------------------------------------------------
# 8. stack_trace_java -- L3 summarizes deep trace
# ------------------------------------------------------------------
$body = @'
Exception in thread "http-nio-8080-exec-3" java.lang.RuntimeException: Database connection lost
    at com.example.api.service.UserService.findById(UserService.java:142)
    at com.example.api.service.UserService$$FastClassBySpringCGLIB$$abc123.invoke(<generated>)
    at org.springframework.cglib.proxy.MethodProxy.invoke(MethodProxy.java:218)
    at org.springframework.aop.framework.CglibAopProxy$CglibMethodInvocation.invokeJoinpoint(CglibAopProxy.java:783)
    at org.springframework.aop.framework.ReflectiveMethodInvocation.proceed(ReflectiveMethodInvocation.java:163)
    at org.springframework.aop.framework.CglibAopProxy$CglibMethodInvocation.proceed(CglibAopProxy.java:753)
    at org.springframework.transaction.interceptor.TransactionInterceptor$1.proceedWithInvocation(TransactionInterceptor.java:123)
    at org.springframework.transaction.interceptor.TransactionAspectSupport.invokeWithinTransaction(TransactionAspectSupport.java:388)
    at org.springframework.transaction.interceptor.TransactionInterceptor.invoke(TransactionInterceptor.java:119)
    at org.springframework.aop.framework.ReflectiveMethodInvocation.proceed(ReflectiveMethodInvocation.java:186)
    at com.example.api.controller.UserController.getUser(UserController.java:67)
    at jdk.internal.reflect.GeneratedMethodAccessor42.invoke(Unknown Source)
    at jdk.internal.reflect.DelegatingMethodAccessorImpl.invoke(DelegatingMethodAccessorImpl.java:43)
    at java.lang.reflect.Method.invoke(Method.java:566)
    at org.springframework.web.method.support.InvocableHandlerMethod.doInvoke(InvocableHandlerMethod.java:205)
    at org.springframework.web.method.support.InvocableHandlerMethod.invokeForRequest(InvocableHandlerMethod.java:150)
    at org.springframework.web.servlet.mvc.method.annotation.ServletInvocableHandlerMethod.invokeAndHandle(ServletInvocableHandlerMethod.java:117)
    at org.springframework.web.servlet.mvc.method.annotation.RequestMappingHandlerAdapter.invokeHandlerMethod(RequestMappingHandlerAdapter.java:895)
    at org.springframework.web.servlet.mvc.method.annotation.RequestMappingHandlerAdapter.handleInternal(RequestMappingHandlerAdapter.java:808)
    at org.springframework.web.servlet.mvc.method.AbstractHandlerMethodAdapter.handle(AbstractHandlerMethodAdapter.java:87)
    at org.springframework.web.servlet.DispatcherServlet.doDispatch(DispatcherServlet.java:1070)
    at org.springframework.web.servlet.DispatcherServlet.doService(DispatcherServlet.java:963)
    at org.springframework.web.servlet.FrameworkServlet.processRequest(FrameworkServlet.java:1006)
    at org.springframework.web.servlet.FrameworkServlet.doGet(FrameworkServlet.java:898)
    at javax.servlet.http.HttpServlet.service(HttpServlet.java:658)
    at org.springframework.web.servlet.FrameworkServlet.service(FrameworkServlet.java:883)
    at javax.servlet.http.HttpServlet.service(HttpServlet.java:765)
    at org.apache.catalina.core.ApplicationFilterChain.internalDoFilter(ApplicationFilterChain.java:227)
    at org.apache.catalina.core.ApplicationFilterChain.doFilter(ApplicationFilterChain.java:162)
Caused by: java.sql.SQLException: Connection to db.internal:5432 refused
    at com.mysql.cj.jdbc.exceptions.SQLError.createCommunicationsException(SQLError.java:164)
    at com.mysql.cj.jdbc.exceptions.SQLExceptionsMapping.translateException(SQLExceptionsMapping.java:64)
    at com.mysql.cj.jdbc.ConnectionImpl.createNewIO(ConnectionImpl.java:1615)
    at com.mysql.cj.jdbc.ConnectionImpl.<init>(ConnectionImpl.java:635)
    at com.mysql.cj.jdbc.ConnectionImpl.getInstance(ConnectionImpl.java:241)
    at com.mysql.cj.jdbc.NonRegisteringDriver.connect(NonRegisteringDriver.java:199)
    at com.zaxxer.hikari.util.DriverDataSource.getConnection(DriverDataSource.java:137)
    at com.zaxxer.hikari.pool.PoolBase.newConnection(PoolBase.java:364)
    at com.zaxxer.hikari.pool.PoolBase.newPoolEntry(PoolBase.java:206)
    at com.zaxxer.hikari.pool.HikariPool.createPoolEntry(HikariPool.java:476)
    ... 28 more
'@
Write-Fixture 'stack_trace_java' $body @{
    category='stack_trace'; expected_layer=3; min_ratio=0.40
    command='cat app.log'
    description='Deep Java stack trace -- L3 summary expected.'
}

Write-Host ''
Write-Host 'Generated fixtures:'
Get-ChildItem $out -Filter '*.txt' | Sort-Object Name | ForEach-Object {
    '  {0,-40} {1,6} bytes' -f $_.Name, $_.Length
}
