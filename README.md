# simpleKvm

같은 네트워크의 Windows PC와 Mac 사이에서 **마우스·키보드·클립보드를 공유**하는 소프트웨어 KVM입니다.
Windows PC에 연결된 키보드/마우스로, 화면 가장자리를 넘어가면 Mac을 제어합니다. 양쪽 모두 **GUI 앱**입니다.

```
[Windows PC (서버, GUI)] ──── LAN (TCP) ────> [Mac (클라이언트, 메뉴바 앱)]
  물리 키보드/마우스                              입력 주입 (CGEvent)
  저수준 훅으로 캡처                              + 클립보드 동기화
```

## 동작 방식

- Windows에서 마우스 커서를 **Mac이 있는 쪽 화면 가장자리** 너머로 밀면 제어권이 Mac으로 넘어갑니다.
- 그 동안 Windows에서는 모든 입력이 차단(swallow)되고 TCP로 Mac에 전달됩니다.
- Mac 화면에서 반대편 가장자리(Windows 쪽)로 커서를 밀면 제어권이 다시 Windows로 돌아옵니다.
- 한쪽에서 텍스트를 복사하면 반대쪽 클립보드에도 반영됩니다(양방향, 텍스트).
- **한/영 키**는 Mac의 입력 소스 전환(Ctrl+Space)으로, **한자 키**는 한자 변환(Option+Return)으로
  전달됩니다. macOS 시스템 설정 → 키보드 → 단축키 → 입력 소스에서 "이전 입력 소스 선택"(^Space)이
  켜져 있어야 합니다 (기본값).
- 비상 복귀 단축키: **Ctrl+Alt+F12** (Windows에서 강제로 제어권 회수)
- 연결이 끊기면 자동으로 Windows 제어로 복귀합니다. `Ctrl+Alt+Del`은 훅으로 가로챌 수 없으므로 항상 탈출구가 됩니다.

## 요구사항

- 양쪽 모두 Rust 툴체인 (https://rustup.rs)
- **Mac: Apple Silicon(arm64) 전용** (Intel Mac 미지원)
- Windows: 10/11

## 빌드 & 실행

### 1. Windows 서버

```bash
cargo build --release -p kvm-server
./target/release/kvm-server.exe
```

GUI 창에서 설정 후 **시작**:

- **Mac 위치**: 이 PC 화면 기준 Mac이 있는 쪽 (왼쪽 / 오른쪽)
- **포트**: 기본 24800
- **이름**: 핸드셰이크 시 클라이언트에 표시될 이름

처음 실행 시 Windows 방화벽 창이 뜨면 **개인 네트워크 허용**을 선택하세요.
설정은 `%APPDATA%\simpleKvm\server.json`에 저장됩니다. `모니터 배치 보기`로 인식 상태를 확인할 수 있습니다.

### 2. Mac 클라이언트

앱 번들(`simpleKvm.app`)로 빌드합니다:

```bash
# (권장, 최초 1회) 재빌드해도 권한이 유지되도록 자체 서명 인증서 생성
./scripts/create-signing-cert.sh

# 빌드 + 번들 (dist/simpleKvm.app 생성)
./scripts/bundle-mac.sh

open dist/simpleKvm.app
```

메뉴바(상태표시줄)에 아이콘이 뜹니다. 아이콘 클릭 → **설정 열기**에서 서버를 지정하고 **연결**하세요.
서버가 실행 중이면 설정 화면의 **"네트워크에서 발견된 서버"** 목록에 자동으로 나타나며(mDNS),
**사용** 버튼으로 주소·포트가 채워집니다. 목록에 안 뜨면 LAN IP를 직접 입력하면 됩니다.
설정은 `~/Library/Application Support/simpleKvm/client.json`에 저장됩니다.

메뉴바 메뉴: **설정 열기 / 로그인 시 자동 실행 / 종료**. 창을 닫으면 종료되지 않고 메뉴바로 숨습니다.

## macOS 권한 (필수)

Mac 클라이언트는 두 가지 권한이 필요합니다:

1. **손쉬운 사용(Accessibility)** — 입력 주입용. 없으면 연결돼도 마우스/키보드가 주입되지 않습니다.
   앱 상단에 빨간 경고 배너가 뜨면 버튼으로 설정을 열어 simpleKvm을 켜세요.
   (시스템 설정 → 개인정보 보호 및 보안 → 손쉬운 사용)
2. **로컬 네트워크(Local Network)** — LAN의 Windows 서버에 연결하기 위해 필요 (macOS 15+).
   권한이 없으면 연결이 `No route to host`로 조용히 실패합니다. 연결 시 뜨는 팝업을 허용하거나,
   시스템 설정 → 개인정보 보호 및 보안 → 로컬 네트워크에서 simpleKvm을 켜세요.

> 참고: 손쉬운 사용 권한은 앱의 코드 서명에 묶입니다. `.app`을 **재빌드하면 서명이 바뀌어 권한이
> 무효화**되는데, `create-signing-cert.sh`로 만든 안정 서명을 쓰면 재빌드해도 권한이 유지됩니다.
> 문제 진단 절차는 [`docs/mac-connection-check.md`](docs/mac-connection-check.md) 참고.

## 옵션

| 옵션 | 대상 | 설명 |
|------|------|------|
| 포트 | 양쪽 | TCP 포트 (기본 24800) |
| Mac 위치 (왼쪽/오른쪽) | 서버 | Windows 화면 기준 Mac의 위치 |
| 마우스 감도 | 클라이언트 | 마우스 이동 배율 (기본 1.0) |
| Windows Ctrl → Mac Command | 클라이언트 | Ctrl+C → Cmd+C 매핑. 기본은 물리 매핑(Ctrl→Control, Win→Command, Alt→Option) |
| 로그인 시 자동 실행 | 클라이언트 | 로그인 시 자동 시작 (LaunchAgent) |

## 다중 모니터

양쪽 모두 다중 모니터를 지원합니다:

- **Windows**: 전환 엣지는 가상 데스크톱의 가장 바깥 좌/우 가장자리입니다. 엣지를 소유한
  모니터의 실제 좌표 기준으로 세로 위치를 매핑하므로, 해상도·배치(가로/세로 회전 포함)가
  달라도 진입/복귀 위치가 어긋나지 않습니다. `모니터 배치 보기`로 인식 상태를 확인하세요.
- **Mac**: 모든 활성 디스플레이의 배치를 인식하며, 커서가 디스플레이 사이를 자유롭게 이동합니다.
  배치상 빈 공간(dead zone)으로는 빠지지 않도록 클램핑됩니다. Windows로의 복귀는 전체 배치의
  가장 바깥 엣지에서만 일어납니다.

## 개발용 도구

Mac 없이 서버를 테스트하려면 가짜 클라이언트를 사용하세요:

```bash
cargo run -p kvm-protocol --example fake_client -- 127.0.0.1 10
```

핸드셰이크 후 10초간 서버가 보내는 메시지(Enter, MouseMove, Key, ...)를 출력합니다.

Mac 없이 서버 코드를 컴파일 검증하려면 (링커 불필요):

```bash
rustup target add x86_64-pc-windows-gnu
cargo check --target x86_64-pc-windows-gnu -p kvm-server
```

## 구조

```
crates/
├── kvm-protocol/   # 공유 메시지 타입 + 프레이밍 (u32 length prefix + bincode)
│   └── src/lib.rs  #   Message enum, clipboard 동기화 헬퍼(arboard, feature="clipboard")
├── kvm-server/     # Windows: 훅 캡처 + TCP 서버 + egui GUI
│   ├── app.rs      #   egui 설정/상태 UI
│   ├── engine.rs   #   훅 스레드 + 네트 스레드 시작/중지
│   ├── hooks.rs    #   WH_MOUSE_LL/WH_KEYBOARD_LL 훅, 원격 모드, 커서 재중심화
│   ├── net.rs      #   accept/핸드셰이크/이벤트 전달/하트비트/클립보드
│   └── config.rs   #   server.json 로드/저장
└── kvm-client/     # macOS: TCP 수신 → CGEvent 주입 + 메뉴바 egui 앱
    ├── app.rs      #   egui 설정/상태 UI
    ├── worker.rs   #   연결/재연결 워커 스레드 + 클립보드
    ├── inject.rs   #   가상 커서/버튼/모디파이어/스크롤 주입
    ├── keymap.rs   #   Windows VK 코드 → macOS 키코드
    ├── tray.rs     #   메뉴바 아이콘/메뉴
    ├── autostart.rs#   로그인 시 자동 실행 (LaunchAgent)
    ├── permission.rs #  손쉬운 사용 권한 확인
    └── config.rs   #   client.json 로드/저장
```

### 프로토콜 요약 (v2)

- 클라이언트가 접속해 `Hello` → 서버가 `HelloAck` 응답 (버전 확인, 현재 v2)
- 서버 → 클라이언트: `Enter{edge, y_ratio}`(제어권 이동), `MouseMove{dx,dy}`, `MouseButton`, `Wheel`, `Key{vk}`, `Leave`, `Heartbeat`(2초)
- 클라이언트 → 서버: `ReturnToServer{y_ratio}` (커서가 복귀 엣지에 닿음)
- 양방향: `Clipboard{text}` (클립보드 텍스트 변경)

## 알려진 제한 / 로드맵

- [x] ~~클립보드 동기화~~ (텍스트, 양방향)
- [x] ~~트레이 아이콘 / GUI 설정~~ (Mac 메뉴바 앱 + Windows GUI)
- [x] ~~한/영 전환 키 매핑~~ (한/영 → Ctrl+Space, 한자 → Option+Return)
- [x] ~~Windows 서버 트레이/자동 시작~~ (상태별 아이콘 색상 포함)
- [x] ~~mDNS 자동 탐색~~ (서버 광고 + 클라이언트 발견 목록. 안 보이면 Windows 방화벽에서
      kvm-server의 UDP 5353 인바운드 허용 필요: `netsh advfirewall firewall add rule
      name="simpleKvm mDNS" dir=in action=allow protocol=UDP localport=5353`)
- [ ] 클립보드 이미지/파일 동기화 (현재는 텍스트만)
- [ ] 원격 모드 중 Windows 커서가 화면 중앙에 보임 (Raw Input + 커서 숨김으로 개선 예정)
- [ ] 전송 암호화(TLS) — 현재 평문이므로 신뢰할 수 있는 LAN에서만 사용
- [x] ~~Mac 다중 모니터~~
- [x] ~~Windows 다중 모니터~~
```
