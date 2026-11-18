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

Requisitos: Rust (MSVC). O app principal agora é **nativo em GPUI** (o framework de interface do editor Zed, desenhado direto na GPU, sem navegador embutido).

```bash
cargo run -p markii-gpui            # app GPUI (principal)
cargo build -p markii-gpui --release
```

No build *release* o GPUI compila shaders com o `fxc.exe` do Windows SDK. O caminho está em `.cargo/config.toml` (`GPUI_FXC_PATH`); ajuste se o seu SDK estiver em outro lugar.

A versão antiga com interface web (Tauri) continua no repositório:

```bash
npm install
npm run dev
```

## Estrutura

```
crates/markii-core/   coleta de dados, sem interface
  lib.rs              Collector: sysinfo + NVML + HWiNFO + clocks reais + alertas
  hwinfo.rs           memória compartilhada do HWiNFO
  hw.rs               mapeia sensores do HWiNFO
  cpufreq.rs          clock real por núcleo (contadores PDH do Windows)
  pcinfo.rs           identificação do PC (WMI) e detecção do cooler
  alerts.rs           configuração e motor de alertas
crates/markii-gpui/   app nativo GPUI
  view.rs             painel (CPU, GPU, Este PC, refrigeração, alertas, configurações)
  draw.rs             gráficos e sprites desenhados (barras, linhas, ventoinhas, coolers)
  theme.rs            paleta grafite + âmbar
  main.rs             janela, bandeja, notificações, thread de coleta
src-tauri/ + ui/      versão Tauri (interface web), usa o mesmo markii-core
```

As duas versões leem o mesmo `%APPDATA%\com.akaineko.markii\config.json`.

## Sprint 2

- **Identidade visual própria**: grafite + âmbar, fonte Bahnschrift, gráficos em barras.
- **Este PC**: fabricante/modelo, placa-mãe, BIOS, Windows, CPU, cada pente de RAM, GPU e driver, modelo de cada disco, com dicas (ex.: RAM em single channel).
- **Refrigeração**: detecta water cooler ou air cooler (bomba nos sensores do HWiNFO ou controlador USB de AIO conhecido) e mostra o desenho correspondente com ventoinhas/bomba animadas. Dá para escolher manualmente nas configurações.
- **Alertas**: CPU, GPU, RAM e discos acima do limite por X segundos → faixa vermelha no app + notificação do Windows, com intervalo para não repetir.
- **Bandeja**: fechar a janela minimiza para a bandeja; o ícone mostra CPU/GPU/RAM no tooltip; clique abre/esconde; menu com "Sair".
- Configurações salvas em `%APPDATA%\com.akaineko.markii\config.json`.

## Próximos sprints

- Sprint 3: histórico gravado em disco e gráficos de longo prazo
- Sprint 4: modo compacto / overlay sempre no topo
