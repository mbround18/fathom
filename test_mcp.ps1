$body = @{
    jsonrpc = "2.0"
    id = 1
    method = "tools/list"
} | ConvertTo-Json

$response = Invoke-WebRequest -Uri 'http://127.0.0.1:18288/mcp' -Method POST -ContentType 'application/json' -Body $body
Write-Output $response.Content
