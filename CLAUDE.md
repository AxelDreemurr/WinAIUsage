# WinAIUsage

App de bandeja del sistema para Windows que monitorea el uso de herramientas de AI: Claude Code, Codex y Antigravity.

## Stack
- Tauri 2 (Rust) — tray icon, acceso al sistema de archivos, lógica de providers
- React + TypeScript — UI del popup
- npm

## Objetivo
Mostrar un popup desde el tray con: tokens usados, costo estimado y quota 
restante para cada herramienta, con diseño moderno tipo Windows 11.

## Estructura
- src/ — React frontend (UI del popup)
- src-tauri/src/ — lógica Rust (providers, tray, comandos)

## Fuentes de datos
- Claude Code: %USERPROFILE%\.claude\projects\**\*.jsonl
- Codex: %USERPROFILE%\.codex\auth.json + sessions\*.jsonl  
- Antigravity: language server local localhost:<puerto dinámico>, 
  endpoint POST GetUserStatus RPC

## Convenciones
- Cada provider es un módulo Rust separado en src-tauri/src/providers/
- Providers exponen un comando Tauri: #[tauri::command]
- UI llama los comandos con invoke() de @tauri-apps/api
- Verificar que compila antes de terminar cada tarea
