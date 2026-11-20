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
  linux.rs            sensores no Linux (hwmon, /proc/stat, RAPL)
  pcinfo_linux.rs     identificação do PC no Linux (/sys, lspci, dmidecode)
  alerts.rs           configuração e motor de alertas
  rgb.rs              controle de RGB: cliente do SDK do OpenRGB, processo e reaplicar no login
  snapshot.rs         resumo dos números para o overlay de VR e o app Android
  remote.rs           servidor só leitura na rede local para o app Android (PIN + token)
crates/markii-gpui/   app nativo GPUI
  view.rs             layout, cabeçalho, Este PC, refrigeração, temperaturas, configurações
  panels.rs           CPU (núcleos/threads), GPU, memória, placa-mãe, ventoinhas, aviso de sensores
  draw.rs             gráficos e sprites desenhados (barras, linhas, ventoinhas, coolers)
  theme.rs            temas (grafite, roxo, carmesim, azul profundo, pastel) e troca de cores
  rgb_view.rs         janela de controle de RGB
  vr.rs               painel no SteamVR (processo --vr-overlay, OpenVR)
  main.rs             janela, bandeja, notificações, thread de coleta
sensors/              auxiliar .NET com LibreHardwareMonitorLib (roda com admin)
src-tauri/ + ui/      versão Tauri (interface web), usa o mesmo markii-core
android/              app Android (Quest, Pico, celular) que lê o Mark II pela rede local
```

As duas versões leem o mesmo `%APPDATA%\com.akaineko.markii\config.json`.

## Sprint 2

- **Identidade visual própria**: grafite + âmbar, fonte Bahnschrift, gráficos em barras.
- **Este PC**: fabricante/modelo, placa-mãe, BIOS, Windows, CPU, cada pente de RAM, GPU e driver, modelo de cada disco, com dicas (ex.: RAM em single channel).
- **Refrigeração**: detecta water cooler ou air cooler (bomba nos sensores do HWiNFO ou controlador USB de AIO conhecido) e mostra o desenho correspondente com ventoinhas/bomba animadas. Dá para escolher manualmente nas configurações.
- **Alertas**: CPU, GPU, RAM e discos acima do limite por X segundos → faixa vermelha no app + notificação do Windows, com intervalo para não repetir.
- **Bandeja**: o ícone mostra CPU/GPU/RAM no tooltip; clique traz a janela para frente; menu com "Sair". Fechar a janela encerra o app (desde a v0.6.1).
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

## v0.5.1

- Temas: **Grafite** (padrão), **Roxo** (preto com roxo escuro), **Lavanda** e **Menta** (pastel escuros), **Algodão-doce** e **Pistache** (pastel claros). Escolha em Configurações → Tema; muda na hora e fica salvo.

## v0.5.2

- **Linux (x64)**: versão nativa GPUI. Sensores direto do kernel (hwmon/lm-sensors), sem driver extra nem administrador; GPU NVIDIA pelo NVML e AMD pelo amdgpu. Potência da CPU (RAPL) e marca/slot de cada pente (dmidecode) só como root. Sem ícone na bandeja. O build é feito pelo GitHub Actions (`.github/workflows/linux.yml`), que anexa o `.tar.gz` ao release.
- Correção: no GPUI os logos SVG saíam com vermelho e azul trocados (o logo da Intel aparecia laranja).

## v0.6.0

- **Controle de RGB** (versão GPUI, Windows e Linux): botão **RGB** no topo abre a janela com memória RAM, placa de vídeo, placa-mãe, cooler (water ou air) e ventoinhas/fitas. Escolha cor, brilho e efeito por dispositivo e clique em Aplicar.
  - Usa o [OpenRGB](https://codeberg.org/OpenRGB/OpenRGB) 1.1 (GPL-2.0), que vem junto no zip (`openrgb/`) e no tar.gz (`OpenRGB.AppImage`); nada é baixado ao rodar. O Mark II roda o OpenRGB escondido como servidor local (127.0.0.1:6742) e conversa pelo SDK.
  - **Cores mantidas sem abrir o app**: grava na memória do dispositivo quando ele suporta e salva o perfil "MarkII"; "Reaplicar ao ligar o PC" cria a tarefa **Mark II RGB** no Agendador do Windows (com administrador, para RAM/GPU via SMBus) ou `~/.config/autostart/markii-rgb.desktop` no Linux.
  - Windows: RAM, GPU e placa-mãe precisam do driver PawnIO e de administrador. Linux: regras udev do OpenRGB e `i2c-dev` (veja o LEIA-ME).
- Temas novos (só GPUI): **Carmesim** (preto com vermelho carmesim) e **Azul profundo**.

## v0.6.1

- **Nada fica rodando em segundo plano**: fechar a janela encerra o Mark II, o auxiliar de sensores e o OpenRGB (o ícone da bandeja só mostra os números e reabre a janela; não minimiza mais para a bandeja).
- O OpenRGB só roda enquanto a janela de RGB está aberta; ao fechá-la ele é encerrado e as cores continuam no hardware.
- O OpenRGB roda dentro de um *job* do Windows ligado ao Mark II (`markii-sensors.exe --openrgb`): se o Mark II for fechado à força ou travar, o Windows encerra o OpenRGB junto. No Linux, o kernel faz o mesmo (`PR_SET_PDEATHSIG`).
- A tarefa de login "Mark II RGB" só aplica o perfil e fecha o OpenRGB em seguida.

## v0.6.2

- **Cores voltam ao reiniciar o PC.** O OpenRGB 1.1 recusa "salvar perfil" vindo do SDK, então a v0.6.0/0.6.1 não tinha o que reaplicar. Agora o Mark II guarda cor, efeito e brilho de cada dispositivo em `rgb.json` (pasta de configurações) ao clicar em Aplicar, e a tarefa de login roda `MarkII.exe --apply-rgb`: sem janela, liga o OpenRGB, espera os dispositivos, reaplica, fecha o OpenRGB e sai. Cada execução fica anotada em `rgb-login.log`.
- Botão **Excluir tarefa "Mark II RGB"** na janela de RGB.
- A janela se ajusta à tela (no máximo 90% da largura e 80% da altura) e abre centralizada.

## v0.7.0

- **Painel no SteamVR**: botão **VR** no topo (ou Configurações → Realidade virtual) mostra CPU, GPU, RAM, ventoinhas e alertas no pulso esquerdo dentro do SteamVR (Quest via Link/Air Link/Virtual Desktop, Index, Vive...). Desligado por padrão.
  - Roda num processo separado (`markii --vr-overlay`) que recebe os números do Mark II pelo stdin: não abre o SteamVR sozinho, se solta quando o SteamVR fecha e termina quando o Mark II termina. Desligado, não roda nada.
  - Usa o `openvr_api` oficial da Valve (OpenVR 2.15.6, BSD-3) pela API C; o painel é desenhado em software (tiny-skia + ab_glyph).
- **App Android** (`android/`, Kotlin, sem bibliotecas extras): janela 2D para Quest 3/3S, Quest 2/Pro, Pico 4 e celulares. Instalação por sideload (adb, SideQuest ou modo desenvolvedor do Pico). O APK é gerado pelo GitHub Actions (`.github/workflows/android.yml`).
- **Acesso pela rede local** para o app (Configurações → Celular e óculos), desligado por padrão: só com a janela do Mark II aberta, só rede local, pareamento com PIN de 6 dígitos (troca a cada pareamento; 5 erros trocam o PIN e pausam 30 s), token por aparelho e só leitura (`/v1/hello`, `/v1/pair`, `/v1/snapshot`). "Desconectar todos" remove os aparelhos pareados.
  - **Liberar no firewall**: se o Windows estiver bloqueando, o botão troca as regras do `markii.exe` por uma só (TCP, porta do Mark II, só a sub-rede local), com um UAC.
- **Óculos / celular no PC**: o app manda o status do próprio aparelho junto com cada leitura (bateria e se está carregando, temperatura da bateria, aquecimento do sistema, memória, armazenamento livre, sinal do Wi-Fi, tempo ligado) e o Mark II mostra num painel "Óculos / celular". Só números de status; dá para desligar no app.
- **Temas no app Android**: Material You (cores do papel de parede, Android 12+) e os 8 temas do Mark II, no botão **Tema**.
- O APK é assinado sempre com a mesma chave (segredo do GitHub), então as atualizações instalam por cima.

## v0.7.1

- **Criar log** (Configurações → Log): o Mark II guarda os eventos da sessão só na memória (sensores, RGB, painel VR, rede local, pareamento, firewall) e, quando você clica, salva `markii-log-<data>.txt` na pasta de configuração e abre a pasta. Nada é gravado sozinho; o arquivo não inclui PIN nem chaves de pareamento.
- Janela cabe na tela também com escala de 125%/150% (notebooks): o tamanho vem da área útil do Windows, sem a barra de tarefas.
- GPU integrada (Intel HD/UHD/Iris, APUs AMD): sem "chips de VRAM" inventados; mostra a temperatura do chip do processador, consumo, clock e "memória compartilhada".

## v0.7.2

- **App Android com 4 abas**: **Painel** (anéis de CPU/GPU/RAM no celular em pé; três colunas grandes na janela deitada do Quest/Pico), **Detalhes** (carga de cada thread, GPU, RAM, discos, ventoinhas, rede), **Este aparelho** (bateria, aquecimento, memória, armazenamento e Wi-Fi do próprio celular/óculos) e **Ajustes** (conexão, tema, enviar status ao PC, desconectar).
- O Mark II do PC passa a enviar a carga de cada thread e o uso dos discos para o app (apps antigos continuam funcionando).

## v0.7.3

- **Baterias do SteamVR**: com o modo VR ligado, o Mark II lê a bateria do óculos, dos controles e dos rastreadores pelo SteamVR (a cada 10 s) e mostra num cartão "SteamVR" no PC, no topo do painel do pulso (`Óculos 62%  E 80%  D 75%`, `+` = carregando) e na aba Detalhes do app Android.
- Correções de detecção:
  - Dispositivos de áudio, rede e periféricos não são mais vistos como water cooler (o VoiceMeeter "VAIO" contava como "AIO"; teclados/headsets Corsair e placas Aquantia também). Agora só nomes de produtos de refrigeração líquida contam.
  - O conector CPU_OPT só conta como bomba em rotação de bomba (≥ 2200 rpm); em rotação de ventoinha é um segundo fan do air cooler.
  - Processadores AMD antigos (FX "Eight-Core", Athlon "Dual-Core", Phenom) não aparecem mais como Intel.
  - Adaptadores de vídeo virtuais (Virtual Desktop, Parsec, área de trabalho remota...) não são mais escolhidos como a GPU monitorada.
  - O indicador NVML só fica verde com uma placa NVIDIA.
  - Placas com chipset Intel H310/H410/H510/H610 mostram 2 slots de RAM (a BIOS de muitas informa 4).
- Painel da CPU sem espaço vazio em processadores com poucos núcleos.

## v0.7.4 (pré-lançamento)

- **Water cooler em placas que não dão nome aos conectores** ("Fan #1", "Fan #2"...): um conector girando entre 2000 e 5500 rpm conta como bomba (palpite).
- Mais controladores reconhecidos: ASUS ROG Ryujin/Ryuo/Strix LC, TUF Gaming LC, AORUS Waterforce, Corsair iCUE LINK e H100x/H150x, Phanteks Glacier.
- O log passa a incluir a placa-mãe, a refrigeração detectada (e o motivo), cada ventoinha com rpm e os controladores de refrigeração encontrados.

## Próximos sprints

- Sprint 3: histórico gravado em disco e gráficos de longo prazo
- Sprint 4: modo compacto / overlay sempre no topo
