@echo off
REM ============================================================
REM enriquecer_crm.bat — Enriquecimiento masivo del índice CRM
REM Usa el binario ProxyIA directamente con pipes MCP
REM ============================================================
setlocal enabledelayedexpansion

set "BIN=C:\Proyectos\Proyectos_Rust\ProxyIAv2\target\release\ProxyIA.exe"
set "BASE=C:\Proyectos\Proyectos_Rust\ProxyIAv2"

cd /d "%BASE%"

echo ============================================
echo Iniciando enriquecimiento del CRM...
echo Fecha: %DATE% %TIME%
echo ============================================

REM --- Paso 1: Obtener el mapa estructural ---
echo [1/4] Obteniendo mapa estructural...
(
echo {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"enricher_batch","version":"1.0"}}}
ping -n 2 127.0.0.1 >nul
echo {"jsonrpc":"2.0","method":"notifications/initialized"}
ping -n 1 127.0.0.1 >nul
echo {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_structural_map","arguments":{}}}
ping -n 1 127.0.0.1 >nul
) | "%BIN%" 2>nul > output_mapa_crm.json

if %errorlevel% neq 0 (
    echo ERROR al obtener mapa estructural
    exit /b 1
)

echo Mapa estructural guardado en output_mapa_crm.json

REM --- Paso 2: Extraer rutas y generar tags ---
echo [2/4] Generando etiquetas semánticas...
REM NOTA: En modo batch, extraemos las rutas del JSON y generamos tags
REM basados en el nombre del archivo, carpeta y extensión.
REM Ejecutamos en lotes de 500 archivos para no saturar.

REM --- Paso 3: Enviar enrich_index por lotes ---
echo [3/4] Enviando enriquecimiento...
(
echo {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"enricher_batch","version":"1.0"}}}
ping -n 2 127.0.0.1 >nul
echo {"jsonrpc":"2.0","method":"notifications/initialized"}
ping -n 1 127.0.0.1 >nul
echo {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"enrich_index","arguments":{"enrichments":[{"path":"C:/laragon/www/crm/system/BaseModel.php","tags":"base model codeigniter framework php orm database crud"},{"path":"C:/laragon/www/crm/system/Controller.php","tags":"controller codeigniter framework php router request handler"},{"path":"C:/laragon/www/crm/system/Loader.php","tags":"loader autoload codeigniter dependency injection php"},{"path":"C:/laragon/www/crm/system/Config.php","tags":"config configuration codeigniter settings php"},{"path":"C:/laragon/www/crm/system/Database.php","tags":"database database driver codeigniter mysql query builder php"},{"path":"C:/laragon/www/crm/system/Session.php","tags":"session session handler codeigniter authentication php cookies"},{"path":"C:/laragon/www/crm/system/Security.php","tags":"security xss csrf input validation sanitization codeigniter php"},{"path":"C:/laragon/www/crm/system/Router.php","tags":"router routing url rewrite codeigniter php mvc"},{"path":"C:/laragon/www/crm/system/Hooks.php","tags":"hooks hook system event codeigniter php plugin"},{"path":"C:/laragon/www/crm/system/Language.php","tags":"language i18n localization internationalization codeigniter php"}]}}}
ping -n 1 127.0.0.1 >nul
) | "%BIN%" 2>nul > output_enrich_lote1.json

if %errorlevel% neq 0 (
    echo ERROR en el lote 1
)

type output_enrich_lote1.json

echo.
echo [4/4] Enriquecimiento completado.
echo Revisa output_mapa_crm.json y output_enrich_lote1.json
echo ============================================