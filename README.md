# Mark II · PC Monitor

App desktop para Windows (Tauri 2 + Rust, sem Electron) que mostra o estado do PC em tempo real, no estilo de painel escuro com "sprites" dos componentes: o die da CPU com cada núcleo e a placa de vídeo com VRM, chips de memória, conector e fan.

## O que mostra

| Bloco | Dados | Fonte |
|---|---|---|
| CPU | carga total e por núcleo, clock por núcleo | Windows (sysinfo) |
| CPU | temperatura do pacote e por núcleo, potência, Vcore | HWiNFO (opcional) |
| GPU NVIDIA | carga, temperatura, VRAM, potência e limite, clocks, fan | NVML (driver NVIDIA) |
| Memória | RAM e swap | Windows |
| Armazenamento | espaço usado por disco | Windows |
| Rede | download e upload | Windows |
| Processos | top 6 por CPU | Windows |
| Ventoinhas | RPM da placa-mãe | HWiNFO (opcional) |

O Windows não expõe a temperatura da CPU sem driver. Para ver temperaturas, potência e ventoinhas, abra o **HWiNFO64** em modo *Sensors-only* e ative **Settings → Shared Memory Support**. O app detecta sozinho.

## Rodar

Requisitos: Rust (MSVC), Node.js, WebView2 (já vem no Windows 11).

```bash
npm install
npm run dev      # abre o app
npm run build    # gera o instalador em src-tauri/target/release/bundle/nsis
```

Para ver só o layout no navegador (com dados de demonstração), sirva a pasta `ui/` em qualquer servidor estático.

## Estrutura

```
ui/                 interface (HTML/CSS/JS puro, sem build)
src-tauri/src/
  main.rs           coleta (sysinfo + NVML + HWiNFO) e envia o evento "telemetry"
  hwinfo.rs         leitura da memória compartilhada do HWiNFO
  hw.rs             mapeia sensores do HWiNFO para os campos do painel
```

## Próximos sprints

- Sprint 2: alertas de temperatura, janela compacta/sempre no topo, ícone na bandeja
- Sprint 3: histórico gravado em disco e gráficos de longo prazo
