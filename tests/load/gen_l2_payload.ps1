# Generates a payload that exercises Layer 2 (tokenizer-aware path shortening).
# Pattern: TypeScript compiler errors with deep file paths — L2 compresses paths via BPE.

$paths = @(
    "C:\Users\Alessandro_Mota\Projects\valraw\apps\web\src\components\dashboard\metrics\CompressionChart.tsx"
    "C:\Users\Alessandro_Mota\Projects\valraw\apps\web\src\components\dashboard\widgets\KpiCard.tsx"
    "C:\Users\Alessandro_Mota\Projects\valraw\apps\web\src\hooks\useCompressionMetrics.ts"
    "C:\Users\Alessandro_Mota\Projects\valraw\apps\web\src\lib\api\compression\client.ts"
    "C:\Users\Alessandro_Mota\Projects\valraw\apps\web\src\types\compression.d.ts"
    "C:\Users\Alessandro_Mota\Projects\valraw\apps\mobile\src\screens\DashboardScreen.tsx"
    "C:\Users\Alessandro_Mota\Projects\valraw\packages\shared\src\utils\tokenCounter.ts"
    "C:\Users\Alessandro_Mota\Projects\valraw\packages\shared\src\models\CompressionRecord.ts"
)

$errors = @(
    "error TS2322: Type 'string | undefined' is not assignable to type 'string'."
    "error TS2339: Property 'compressed' does not exist on type 'ApiResponse'."
    "error TS7006: Parameter 'record' implicitly has an 'any' type."
    "error TS2345: Argument of type 'number' is not assignable to parameter of type 'string'."
    "error TS2304: Cannot find name 'CompressionLayer'."
    "error TS2551: Property 'tokensIn' does not exist on type 'SessionSummary'. Did you mean 'total_original_tokens'?"
    "error TS1005: ';' expected."
    "error TS2769: No overload matches this call."
)

$lines = @()

foreach ($path in $paths) {
    $errCount = Get-Random -Min 3 -Max 8
    for ($i = 0; $i -lt $errCount; $i++) {
        $err = $errors[(Get-Random -Min 0 -Max $errors.Count)]
        $line = Get-Random -Min 10 -Max 300
        $col  = Get-Random -Min 1 -Max 60
        $lines += "${path}(${line},${col}): $err"
        $lines += ""
        # Context snippet
        $lines += "$(Get-Random -Min 10 -Max 300) |     const value: string = someFunction(data);"
        $lines += "    |                  ~~~~~~~~~~~~~~~"
        $lines += ""
    }
}

$lines += ""
$lines += "Found $($lines.Count) errors in $($paths.Count) files."
$lines += ""
$lines += "Process exited with exit code 2."
$lines += ""

$lines -join "`n"
