```

aimer/
├─ Cargo.toml  src/  tests/   # umbrella `aimer` stays at the root
│
├─ cores/       rubick  venus  simd  color  shape  ffi_utils  std
├─ contracts/   accessibility  data_view  form  i18n  media          
├─ support/     macro  attribute  events  utils
├─ system/      native  haptics
├─ services/    storage  vault  passkey                              
├─ graphic/     cupid  canvas render
│               cupid_svg  cupid_font  cupid_gpu             # split out of cupid
│               cupid_wgpu  cupid_metal  cupid_vulkan  cupid_opengl  cupid_dx12  cupid_webgl  cupid_webgpu
├─ framework/   widget (facade)  element  portable  widget_core  async_builder  focusable
│               animation  decoration  provider  style  focus  persist quiver
├─ layout/      container  flex  grid  scroll  space
├─ overlay/     modal  ctxmenu
├─ widgets/     svg  lucide  assets  text  input  markdown  custom_paint
├─ controls/    collapsible  indicator  range  selection  picker  feedback  dnd
├─ routing/     router  navigation
├─ host/        platform  window frame_stats  hot_reload anteros
├─ app/jaime/
├─ experiments/ laboratory reload_protocol  reload_server  wasm_guest
├─ docs/        LAYOUT.md  plans/ (was zodiac_plans)  notes from anteros/cupid
└─ scripts/


```