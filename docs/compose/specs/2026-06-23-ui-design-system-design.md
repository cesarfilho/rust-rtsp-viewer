# UI Design System - rust-rtsp-viewer

## [S1] Problem

A interface atual do rust-rtsp-viewer usa cores hardcoded no CSS com valores opacos, tipografia com tamanhos variados sem hierarquia clara, e espaçamentos inconsistentes. Falta um design system que garanta consistência visual e profissionalismo.

## [S2] Solution Overview

Implementar um design system completo com:
1. Sistema de cores semântico com tokens
2. Sistema tipográfico com escala consistente
3. Sistema de espaçamento 4px base
4. Padrões de componentes (navigation, feedback, data display)

## [S3] Color System

### Tokens de Cor

| Token | Uso | Valor |
|-------|-----|-------|
| `--bg-primary` | Fundo janela | `#0a0a0a` |
| `--bg-secondary` | Sidebar, painéis | `#0d0d0d` |
| `--bg-tertiary` | Cards, rows | `#141414` |
| `--border-subtle` | Bordas leves | `#1a1a1a` |
| `--border-default` | Bordas padrão | `#222` |
| `--border-strong` | Bordas destaque | `#333` |
| `--text-primary` | Texto principal | `#d4d4d4` |
| `--text-secondary` | Labels, stats | `#888` |
| `--text-tertiary` | Texto desabilitado | `#525252` |
| `--accent-blue` | Seleção, links | `#60a5fa` |
| `--accent-green` | Online, sucesso | `#22c55e` |
| `--accent-red` | Erro, offline | `#ef4444` |
| `--accent-amber` | Warning, gravando | `#f59e0b` |

### Aplicação

- Backgrounds: usar `--bg-primary` para janela, `--bg-secondary` para painéis laterais
- Bordas: usar `--border-subtle` para separadores leves, `--border-default` para bordas visíveis
- Texto: usar `--text-primary` para conteúdo principal, `--text-secondary` para labels
- Accents: usar cores semânticas para estados (online/offline/error/recording)

## [S4] Typography

### Sistema Tipográfico

| Nível | Uso | Fonte | Tamanho | Peso |
|-------|-----|-------|---------|------|
| Display | Título app | monospace | 10pt | bold |
| Heading | Seções painel | monospace | 9pt | regular |
| Body | Nomes câmeras | monospace | 8pt | regular |
| Caption | Stats, badges | monospace | 7pt | regular |
| Micro | Labels tiny | monospace | 7pt | regular |

### Regras

- Monospace para todo texto técnico (métricas, stats)
- Line-height: 1.4-1.6 para legibilidade
- Contraste mínimo: 4.5:1 para texto normal (WCAG AA)
- Hierarquia visual clara: Display > Heading > Body > Caption > Micro

## [S5] Spacing & Layout

### Escala de Espaçamento (4px base)

| Token | Valor | Uso |
|-------|-------|-----|
| `--space-xs` | 2px | Gap entre badges |
| `--space-sm` | 4px | Padding interno labels |
| `--space-md` | 8px | Margem padrão |
| `--space-lg` | 12px | Gap entre seções |
| `--space-xl` | 16px | Margem externa |
| `--space-2xl` | 24px | Separador visual |

### Regras de Layout

- Grid cells: gap de 2px (compacto) ou 4px (confortável)
- Sidebar row height: 36px fixo
- Panel padding: 8px em todos os lados
- Border-radius: 3px para badges, 6px para cards
- Hit target mínimo: 40x40px para botões clicáveis

## [S6] Navigation Improvements

### Sidebar

- Hover state mais visível (background `--bg-tertiary`)
- Indicador de câmera selecionada (borda esquerda `--accent-blue`)
- Badges de status com cores semânticas (online=green, offline=red, recording=amber)

### Toolbar

- Botões com hover/active states claros
- Ícones Unicode mantidos mas com fallback visual
- Separador visual entre seções

### Icon Bar

- Tooltips em hover
- Indicador ativo mais claro (outline `--accent-green`)
- Tamanho de hit target: 40x40px mínimo

## [S7] Visual Feedback

### Loading States

- Spinner animation para câmeras carregando
- Skeleton placeholder enquanto dados não chegam

### Empty States

- Mensagem amigável quando nenhuma câmera selecionada
- Ilustração simple ou ícone contextual

### Error States

- Bordas vermelhas para câmeras offline (`--accent-red`)
- Toast notifications com cores semânticas
- Retry button quando aplicável

### Success States

- Toast verde para snapshots/gravações (`--accent-green`)
- Badge de áudio ativo (badge verde)

## [S8] Data Display

### Info Panel

- Grid de métricas com alinhamento consistente
- Sparkline com gradiente suave
- VU meter com segmentos coloridos (green → yellow → red)
- Separadores visuais entre seções

### Status Bar

- Informações compactas e legíveis
- Indicador de saúde da stream

## [S9] Implementation Scope

### Arquivos para modificar

1. `src/infrastructure/nvr/css.rs` - CSS tokens e variáveis
2. `src/infrastructure/nvr/constants.rs` - Constantes de espaçamento
3. `src/infrastructure/nvr/cell.rs` - CameraCell component
4. `src/infrastructure/nvr/sidebar.rs` - Sidebar rows
5. `src/infrastructure/nvr/info_panel.rs` - Info panel
6. `src/infrastructure/nvr/mod.rs` - Toolbar, layout

### Ordem de implementação

1. CSS tokens e variáveis (fundação)
2. Sidebar (mais visível)
3. Toolbar (navegação)
4. Info Panel (data display)
5. Camera Cells (grid)
6. Feedback states (toasts, loading)

## [S10] Success Criteria

- [ ] CSS usa tokens de cor em vez de valores hardcoded
- [ ] Tipografia segue escala definida
- [ ] Espaçamentos usam escala 4px
- [ ] Hover/active states visíveis em todos os botões
- [ ] Indicadores de estado claros (online/offline/recording)
- [ ] Contraste mínimo WCAG AA em todo texto
- [ ] Hit targets ≥ 40x40px para elementos clicáveis
- [ ] Zero warnings no build
- [ ] Todos os testes existentes continuam passando
