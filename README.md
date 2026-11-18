# Mark II · PC Monitor

App desktop para Windows (Tauri 2 + Rust, sem Electron) que mostra o estado do PC em tempo real, no estilo de painel escuro com "sprites" dos componentes: o die da CPU com cada núcleo e a placa de vídeo com VRM, chips de memória, conector e fan.

## O que mostra

Tudo é lido do PC de quem está usando: nada é fixo para um modelo de CPU, GPU ou placa.

| Bloco | Dados | Fonte |
|---|---|---|
| Marcas | logo da marca da CPU (Intel/AMD + geração), GPU (NVIDIA, AMD Radeon, Intel Arc), placa-mãe e de cada pente de RAM | nome do hardware + código JEDEC |
| CPU | carga por núcleo, por thread **ou os dois juntos** (botão no painel), clock min/máx atual e da sessão | Windows |
| CPU | temperatura do pacote e por núcleo, Core max, potência (pacote e núcleos), Vcore da placa, VID pedido pela CPU e por núcleo, barramento | sensores (LHM) |
| GPU | uso, temperatura, hotspot, memória, VRM, tensão, consumo e limite, VRAM, clocks, fan (% e rpm) | NVML + sensores |
| Memória | uso, frequência, tipo, tensão, latência (CL-tRCD-tRP-tRAS em ciclos e ns), temperatura do pente, consumo do controlador, canais | WMI + SPD |
| Placa-mãe | slots de RAM usados, conectores de fan usados, VRM, chipset, tensões, controladores RGB USB | SMBIOS + chip de sensores |
| Ventoinhas | quantas existem e quais giram, rpm e % de cada conector | chip de sensores |
| Discos | espaço, temperatura, vida útil, horas ligado | Windows + SMART |
| Rede, processos | download/upload, top 6 por CPU | Windows |

Quando o sensor não existe no hardware (por exemplo, temperatura da memória numa GPU que não tem esse sensor, ou VRM numa placa sem termistor), o painel mostra "—" com o motivo.

### Sensores completos (LibreHardwareMonitor + PawnIO)

CPU, ventoinhas e placa-mãe só podem ser lidos com acesso de kernel. O app usa a [LibreHardwareMonitorLib](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) (MPL-2.0) num processo auxiliar, `markii-sensors.exe`, com o driver assinado [PawnIO](https://pawnio.eu/):

- Ao abrir, o app pede permissão de administrador (UAC) **só para o auxiliar**; a interface continua como usuário comum.
- O auxiliar **só envia** dados por um *named pipe* local; ele não recebe comandos.
- Se o UAC for recusado, aparece um aviso e o auxiliar roda **sem administrador** (GPU e discos continuam funcionando) com um botão "Tentar com administrador".
- Se o PawnIO não estiver instalado, o aviso tem o botão "Instalar PawnIO", que roda o instalador oficial que vem junto do app (`vendor/pawnio/`, GPL-2.0). Nada é baixado da internet.
- O HWiNFO continua funcionando como fonte alternativa, se estiver aberto com *Shared Memory Support*.

## Rodar

Requisitos: Rust (MSVC). O app principal agora é **nativo em GPUI** (o framework de interface do editor Zed, desenhado direto na GPU, sem navegador embutido).

```bash
cargo run -p markii-gpui            # app GPUI (principal)
cargo build -p markii-gpui --release
```

No build *release* o GPUI compila shaders com o `fxc.exe` do Windows SDK. O caminho está em `.cargo/config.toml` (`GPUI_FXC_PATH`); ajuste se o seu SDK estiver em outro lugar.

O auxiliar de sensores é .NET (precisa do SDK .NET 10 só para compilar):

```bash
dotnet publish sensors -c Release -o target/sensors
```

O app procura `markii-sensors.exe` ao lado do executável, em `sensors/` ou em `../sensors/` (o `target/sensors` acima serve para `target/release`). A variável `MARKII_SENSORS` aponta para outro caminho.

Para gerar os zips de release (compila tudo e confere se cada zip tem o auxiliar, o instalador do PawnIO e as licenças):

```bash
powershell -ExecutionPolicy Bypass -File scripts/package.ps1
```

A versão com interface web (Tauri) continua no repositório e tem os mesmos painéis:

```bash
npm install
npm run dev
```

## Estrutura

```
crates/markii-core/   coleta de dados, sem interface
  lib.rs              Collector: sysinfo + NVML + sensores + clocks reais + alertas
  lhm.rs              inicia o auxiliar (UAC), lê o pipe e mapeia os sensores do LHM
  ram.rs              latência em ciclos, mapa de slots, canais
  brands.rs           reconhece a marca de CPU, GPU, placa-mãe e RAM; logos (Simple Icons, CC0)
  hwinfo.rs           memória compartilhada do HWiNFO (fonte alternativa)
  hw.rs               estruturas dos sensores + mapeamento do HWiNFO
  cpufreq.rs          clock real por núcleo (contadores PDH do Windows)
  pcinfo.rs           identificação do PC (WMI) e detecção do cooler
  alerts.rs           configuração e motor de alertas
crates/markii-gpui/   app nativo GPUI
  view.rs             layout, cabeçalho, Este PC, refrigeração, temperaturas, configurações
  panels.rs           CPU (núcleos/threads), GPU, memória, placa-mãe, ventoinhas, aviso de sensores
  draw.rs             gráficos e sprites desenhados (barras, linhas, ventoinhas, coolers)
  theme.rs            paleta grafite + âmbar
  main.rs             janela, bandeja, notificações, thread de coleta
sensors/              auxiliar .NET com LibreHardwareMonitorLib (roda com admin)
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

## Sprint 4

- Sensores próprios (LibreHardwareMonitor + PawnIO), sem depender do HWiNFO.
- CPU: alternar entre núcleos e threads; potência, Vcore, VID, Core max, clock mín./máx.
- GPU compacta com hotspot, memória, VRM, tensão, consumo e clock da memória.
- Painéis novos de Memória RAM e Placa-mãe; ventoinhas com contagem.

## v0.5

- Logo da marca nos painéis: CPU (Intel/AMD, com a geração ou série), GPU (NVIDIA, AMD Radeon, Intel Arc), placa-mãe (ASUS, ROG, MSI, Gigabyte...) e cada pente de RAM (Kingston, Corsair, G.Skill, HP...). Marcas sem logo livre aparecem como um selo de texto.
- CPU: terceira opção "Ambos" mostra cada núcleo com a carga das suas threads.
- Logos: [Simple Icons](https://simpleicons.org) (CC0). As marcas pertencem aos seus donos e só identificam o hardware.

## Próximos sprints

- Sprint 3: histórico gravado em disco e gráficos de longo prazo
- Sprint 4: modo compacto / overlay sempre no topo
