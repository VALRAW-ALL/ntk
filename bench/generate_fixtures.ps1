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

# ------------------------------------------------------------------
# 9. python_django_trace -- Django + gunicorn/asgiref frames
# ------------------------------------------------------------------
$body = @'
Internal Server Error: /api/users/42/profile/
Traceback (most recent call last):
  File "/app/views/users.py", line 87, in get_profile
    profile = user.get_profile_with_preferences()
  File "/app/models/user.py", line 234, in get_profile_with_preferences
    prefs = Preferences.objects.get(user_id=self.id, active=True)
  File "/usr/local/lib/python3.11/site-packages/django/db/models/manager.py", line 85, in manager_method
    return getattr(self.get_queryset(), name)(*args, **kwargs)
  File "/usr/local/lib/python3.11/site-packages/django/db/models/query.py", line 431, in get
    raise self.model.DoesNotExist(
  File "/usr/local/lib/python3.11/site-packages/django/db/models/base.py", line 612, in __init__
    pass
  File "/usr/local/lib/python3.11/site-packages/django/core/handlers/exception.py", line 47, in inner
    response = get_response(request)
  File "/usr/local/lib/python3.11/site-packages/django/core/handlers/base.py", line 181, in _get_response
    response = wrapped_callback(request, *callback_args, **callback_kwargs)
  File "/usr/local/lib/python3.11/site-packages/asgiref/sync.py", line 477, in __call__
    ret = await asyncio.wait_for(future, timeout=None)
  File "/usr/local/lib/python3.11/site-packages/gunicorn/workers/sync.py", line 131, in handle_request
    self.cfg.post_request(self, req, environ, resp)
app.models.Preferences.DoesNotExist: Preferences matching query does not exist.
[2026-04-15 10:23:45,123] ERROR django.request: Internal Server Error: /api/users/42/profile/
'@
Write-Fixture 'python_django_trace' $body @{
    category='stack_trace'; expected_layer=2; min_ratio=0.30
    command='python manage.py runserver'
    description='Django request handler stack trace with asgiref/gunicorn framework frames.'
}

# ------------------------------------------------------------------
# 10. node_express_trace -- Express middleware + node:internal frames
# ------------------------------------------------------------------
$body = @'
TypeError: Cannot read properties of undefined (reading 'accessToken')
    at exchangeCodeForToken (/app/src/auth/oauth.ts:142:28)
    at callbackHandler (/app/src/routes/auth.ts:88:20)
    at Layer.handle [as handle_request] (/app/node_modules/express/lib/router/layer.js:95:5)
    at next (/app/node_modules/express/lib/router/route.js:144:13)
    at Route.dispatch (/app/node_modules/express/lib/router/route.js:114:3)
    at Layer.handle [as handle_request] (/app/node_modules/express/lib/router/layer.js:95:5)
    at /app/node_modules/express/lib/router/index.js:284:15
    at Function.process_params (/app/node_modules/express/lib/router/index.js:346:12)
    at next (/app/node_modules/express/lib/router/index.js:280:10)
    at /app/node_modules/express/lib/router/index.js:235:24
    at Function.handle (/app/node_modules/express/lib/router/index.js:175:3)
    at router (/app/node_modules/express/lib/router/index.js:47:12)
    at Layer.handle [as handle_request] (/app/node_modules/express/lib/router/layer.js:95:5)
    at trim_prefix (/app/node_modules/express/lib/router/index.js:328:13)
    at /app/node_modules/express/lib/router/index.js:286:9
    at Function.process_params (/app/node_modules/express/lib/router/index.js:346:12)
    at next (/app/node_modules/express/lib/router/index.js:280:10)
    at expressInit (/app/node_modules/express/lib/middleware/init.js:40:5)
    at Layer.handle [as handle_request] (/app/node_modules/express/lib/router/layer.js:95:5)
    at trim_prefix (/app/node_modules/express/lib/router/index.js:328:13)
    at /app/node_modules/express/lib/router/index.js:286:9
    at processTicksAndRejections (node:internal/process/task_queues:95:5)
    at async Module.default_1 (node:internal/modules/esm/loader:210:14)
[express] error: Token exchange failed for session abc123def456
'@
Write-Fixture 'node_express_trace' $body @{
    category='stack_trace'; expected_layer=2; min_ratio=0.50
    command='node server.js'
    description='Node.js Express stack trace with heavy node_modules/express and node:internal frames.'
}

# ------------------------------------------------------------------
# 11. go_panic_trace -- Go runtime panic with goroutine dumps
# ------------------------------------------------------------------
$body = @'
panic: runtime error: index out of range [5] with length 3

goroutine 1 [running]:
main.processItems({0xc0000b2000?, 0xc00009e000?, 0x3})
	/home/dev/app/main.go:48 +0x1f5
main.main()
	/home/dev/app/main.go:29 +0x91
runtime.main()
	/usr/local/go/src/runtime/proc.go:267 +0x2bb
runtime.goexit({})
	/usr/local/go/src/runtime/asm_amd64.s:1650 +0x5
runtime.systemstack()
	/usr/local/go/src/runtime/asm_amd64.s:509 +0x4a
runtime.sysargs()
	/usr/local/go/src/runtime/runtime1.go:180 +0x88
runtime.args()
	/usr/local/go/src/runtime/runtime1.go:66 +0x1e
runtime.schedinit()
	/usr/local/go/src/runtime/proc.go:696 +0x8c

goroutine 2 [force gc (idle)]:
runtime.gopark(0x0?, 0x0?, 0x0?, 0x0?, 0x0?)
	/usr/local/go/src/runtime/proc.go:398 +0xce
runtime.goparkunlock(...)
	/usr/local/go/src/runtime/proc.go:404
runtime.forcegchelper()
	/usr/local/go/src/runtime/proc.go:322 +0xb3
runtime.goexit({})
	/usr/local/go/src/runtime/asm_amd64.s:1650 +0x5
created by runtime.init.6 in goroutine 1
	/usr/local/go/src/runtime/proc.go:310 +0x1a

exit status 2
'@
Write-Fixture 'go_panic_trace' $body @{
    category='stack_trace'; expected_layer=2; min_ratio=0.40
    command='go run main.go'
    description='Go runtime panic with runtime.main/runtime.goexit framework frames and secondary goroutine dumps.'
}

# ------------------------------------------------------------------
# 12. php_symfony_trace -- Symfony HttpKernel + /vendor frames
# ------------------------------------------------------------------
$body = @'
Symfony\Component\HttpKernel\Exception\NotFoundHttpException: No route found for "GET /api/users/99/orders"

  at /app/vendor/symfony/http-kernel/EventListener/RouterListener.php:136
  at Symfony\Component\HttpKernel\EventListener\RouterListener->onKernelRequest(object(RequestEvent), 'kernel.request', object(EventDispatcher))
     (/app/vendor/symfony/event-dispatcher/EventDispatcher.php:270)
  at Symfony\Component\EventDispatcher\EventDispatcher->callListeners(array(array(object(RouterListener), 'onKernelRequest')), 'kernel.request', object(RequestEvent))
     (/app/vendor/symfony/event-dispatcher/EventDispatcher.php:230)
  at Symfony\Component\EventDispatcher\EventDispatcher->doDispatch(array(array(object(RouterListener), 'onKernelRequest')), 'kernel.request', object(RequestEvent))
     (/app/vendor/symfony/event-dispatcher/EventDispatcher.php:59)
  at Symfony\Component\EventDispatcher\EventDispatcher->dispatch(object(RequestEvent), 'kernel.request')
     (/app/vendor/symfony/http-kernel/HttpKernel.php:139)
  at Symfony\Component\HttpKernel\HttpKernel->handleRaw(object(Request), 1)
     (/app/vendor/symfony/http-kernel/HttpKernel.php:75)
  at Symfony\Component\HttpKernel\HttpKernel->handle(object(Request), 1, true)
     (/app/vendor/symfony/http-kernel/Kernel.php:197)
  at Symfony\Component\HttpKernel\Kernel->handle(object(Request))
     (/app/public/index.php:27)

#0 /app/vendor/symfony/http-kernel/HttpKernel.php(76): Symfony\Component\HttpKernel\HttpKernel->handleRaw()
#1 /app/vendor/symfony/http-kernel/Kernel.php(197): Symfony\Component\HttpKernel\HttpKernel->handle()
#2 /app/vendor/symfony/runtime/Runner/Symfony/HttpKernelRunner.php(35): Symfony\Component\HttpKernel\Kernel->handle()
#3 /app/vendor/symfony/runtime/Runner/Symfony/ResponseRunner.php(29): Symfony\Component\Runtime\Runner\Symfony\HttpKernelRunner->run()
#4 /app/vendor/autoload_runtime.php(29): Symfony\Component\Runtime\Runner\Symfony\ResponseRunner->run()
#5 /app/public/index.php(5): require_once('...')
#6 {main}

[2026-04-15T10:23:45+00:00] request.CRITICAL: Uncaught PHP Exception Symfony\Component\HttpKernel\Exception\NotFoundHttpException
'@
Write-Fixture 'php_symfony_trace' $body @{
    category='stack_trace'; expected_layer=2; min_ratio=0.35
    command='php bin/console'
    description='Symfony HttpKernel NotFoundHttpException with heavy /vendor/symfony framework frames.'
}

# ------------------------------------------------------------------
# 13. csharp_aspnet_trace -- ASP.NET Core 8 request pipeline exception
# ------------------------------------------------------------------
$body = @'
System.InvalidOperationException: Sequence contains no elements
   at System.Linq.ThrowHelper.ThrowNoElementsException()
   at System.Linq.Enumerable.First[TSource](IEnumerable`1 source)
   at MyApp.Services.OrderService.GetLatest(Int32 userId) in /app/Services/OrderService.cs:line 42
   at MyApp.Controllers.OrdersController.GetLatest(Int32 userId) in /app/Controllers/OrdersController.cs:line 28
   at lambda_method47(Closure , Object , Object[] )
   at Microsoft.AspNetCore.Mvc.Infrastructure.ActionMethodExecutor.SyncActionResultExecutor.Execute(IActionResultTypeMapper mapper, ObjectMethodExecutor executor, Object controller, Object[] arguments)
   at Microsoft.AspNetCore.Mvc.Infrastructure.ControllerActionInvoker.InvokeActionMethodAsync()
   at Microsoft.AspNetCore.Mvc.Infrastructure.ControllerActionInvoker.Next(State& next, Scope& scope, Object& state, Boolean& isCompleted)
   at Microsoft.AspNetCore.Mvc.Infrastructure.ControllerActionInvoker.InvokeNextActionFilterAsync()
   at System.Threading.Tasks.Task.<>c.<ThrowAsync>b__128_0(Object state)
   at System.Threading.Tasks.ThreadPoolTaskScheduler.TryExecuteTaskInline(Task task, Boolean taskWasPreviouslyQueued)
   at Microsoft.AspNetCore.Mvc.Infrastructure.ResourceInvoker.Rethrow(ActionExecutedContextSealed context)
   at Microsoft.AspNetCore.Mvc.Infrastructure.ResourceInvoker.Next(State& next, Scope& scope, Object& state, Boolean& isCompleted)
   at Microsoft.AspNetCore.Mvc.Infrastructure.ResourceInvoker.InvokeFilterPipelineAsync()
   at Microsoft.AspNetCore.Routing.EndpointMiddleware.<Invoke>d__3.MoveNext()
   at Microsoft.AspNetCore.Authorization.AuthorizationMiddleware.Invoke(HttpContext context)
   at Microsoft.AspNetCore.Diagnostics.DeveloperExceptionPageMiddleware.Invoke(HttpContext context)
   at Microsoft.AspNetCore.HostFiltering.HostFilteringMiddleware.Invoke(HttpContext context)
   at Microsoft.AspNetCore.Server.Kestrel.Core.Internal.Http.HttpProtocol.ProcessRequests[TContext](IHttpApplication``1 application)
   at System.Runtime.ExceptionServices.ExceptionDispatchInfo.Throw()
fail: Microsoft.AspNetCore.Server.Kestrel[13] Connection id "0HMV..." request reached an unexpected state
'@
Write-Fixture 'csharp_aspnet_trace' $body @{
    category='stack_trace'; expected_layer=2; min_ratio=0.35
    command='dotnet run'
    description='ASP.NET Core 8 request-pipeline exception with EndpointMiddleware / Kestrel / ExceptionDispatchInfo frames.'
}

# ------------------------------------------------------------------
# 14. typescript_react_trace -- React 18 + webpack bundle + zone.js
# ------------------------------------------------------------------
$body = @'
TypeError: Cannot read properties of null (reading 'getBoundingClientRect')
    at useLayoutEffect (webpack-internal:///./src/components/Modal.tsx:42:23)
    at commitHookEffectListMount (webpack-internal:///./node_modules/react-dom/cjs/react-dom.development.js:23050:26)
    at commitLayoutEffectOnFiber (webpack-internal:///./node_modules/react-dom/cjs/react-dom.development.js:23168:17)
    at commitLayoutMountEffects_complete (webpack-internal:///./node_modules/react-dom/cjs/react-dom.development.js:24689:9)
    at commitLayoutEffects_begin (webpack-internal:///./node_modules/react-dom/cjs/react-dom.development.js:24675:7)
    at commitLayoutEffects (webpack-internal:///./node_modules/react-dom/cjs/react-dom.development.js:24613:3)
    at commitRootImpl (webpack-internal:///./node_modules/react-dom/cjs/react-dom.development.js:26825:5)
    at commitRoot (webpack-internal:///./node_modules/react-dom/cjs/react-dom.development.js:26546:5)
    at finishConcurrentRender (webpack-internal:///./node_modules/react-dom/cjs/react-dom.development.js:25843:9)
    at performConcurrentWorkOnRoot (webpack-internal:///./node_modules/react-dom/cjs/react-dom.development.js:25655:7)
    at workLoop (webpack-internal:///./node_modules/scheduler/cjs/scheduler.development.js:266:34)
    at flushWork (webpack-internal:///./node_modules/scheduler/cjs/scheduler.development.js:239:14)
    at MessagePort.performWorkUntilDeadline (webpack-internal:///./node_modules/scheduler/cjs/scheduler.development.js:533:21)
    at ZoneDelegate.invokeTask (webpack-internal:///./node_modules/zone.js/bundles/zone.umd.js:412:31)
    at Object.onInvokeTask (__zone_symbol__ZoneAwareError.js:2:12)
    at Zone.runTask (webpack-internal:///./node_modules/zone.js/bundles/zone.umd.js:181:47)
Error boundary caught the error: Modal opened before ref was attached
'@
Write-Fixture 'typescript_react_trace' $body @{
    category='stack_trace'; expected_layer=2; min_ratio=0.45
    command='npm run dev'
    description='React 18 + webpack bundle runtime error with heavy react-dom + scheduler + zone.js frames.'
}

# ------------------------------------------------------------------
# 15. kotlin_android_trace -- Android IllegalStateException + coroutines
# ------------------------------------------------------------------
$body = @"
FATAL EXCEPTION: main
Process: com.example.myapp, PID: 12345
java.lang.IllegalStateException: View must be attached to a ViewTree
`tat com.example.myapp.ui.home.HomeFragment.onViewCreated(HomeFragment.kt:58)
`tat com.example.myapp.ui.home.HomeViewModel.loadUsers(HomeViewModel.kt:42)
`tat androidx.fragment.app.Fragment.performViewCreated(Fragment.java:3089)
`tat androidx.fragment.app.FragmentStateManager.createView(FragmentStateManager.java:548)
`tat androidx.fragment.app.FragmentStateManager.moveToExpectedState(FragmentStateManager.java:282)
`tat androidx.fragment.app.FragmentManager.executeOpsTogether(FragmentManager.java:2189)
`tat androidx.lifecycle.LiveData.considerNotify(LiveData.java:133)
`tat androidx.lifecycle.LiveData.dispatchingValue(LiveData.java:151)
`tat androidx.lifecycle.LiveData.setValue(LiveData.java:309)
`tat androidx.lifecycle.MutableLiveData.setValue(MutableLiveData.java:50)
`tat kotlinx.coroutines.DispatchedTask.run(DispatchedTask.kt:108)
`tat kotlinx.coroutines.internal.LimitedDispatcher`$Worker.run(LimitedDispatcher.kt:115)
`tat kotlinx.coroutines.scheduling.TaskImpl.run(Tasks.kt:103)
`tat kotlinx.coroutines.scheduling.CoroutineScheduler.runSafely(CoroutineScheduler.kt:584)
`tat kotlinx.coroutines.scheduling.CoroutineScheduler`$Worker.executeTask(CoroutineScheduler.kt:793)
`tat kotlinx.coroutines.scheduling.CoroutineScheduler`$Worker.runWorker(CoroutineScheduler.kt:697)
`tat kotlinx.coroutines.scheduling.CoroutineScheduler`$Worker.run(CoroutineScheduler.kt:684)
`tat android.os.Handler.dispatchMessage(Handler.java:106)
`tat android.os.Looper.loop(Looper.java:214)
`tat android.app.ActivityThread.main(ActivityThread.java:7356)
`tat java.lang.reflect.Method.invoke(Native Method)
`tat com.android.internal.os.RuntimeInit`$MethodAndArgsCaller.run(RuntimeInit.java:492)
`tat com.android.internal.os.ZygoteInit.main(ZygoteInit.java:930)
`tat dalvik.system.VMRuntime.nativeExit(Native Method)
E/AndroidRuntime(12345): Shutting down VM due to IllegalStateException
"@
Write-Fixture 'kotlin_android_trace' $body @{
    category='stack_trace'; expected_layer=2; min_ratio=0.35
    command='adb logcat'
    description='Android / Kotlin IllegalStateException with androidx fragment lifecycle + kotlinx.coroutines scheduler frames.'
}

Write-Host ''
Write-Host 'Generated fixtures:'
Get-ChildItem $out -Filter '*.txt' | Sort-Object Name | ForEach-Object {
    '  {0,-40} {1,6} bytes' -f $_.Name, $_.Length
}
