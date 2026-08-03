# simpleKvm

같은 네트워크의 Windows PC와 Mac 사이에서 **마우스와 키보드를 공유**하는 소프트웨어 KVM입니다.
Windows PC에 연결된 키보드/마우스로, 화면 가장자리를 넘어가면 Mac을 제어합니다.

```
[Windows PC (서버)] ──── LAN (TCP) ────> [Mac (클라이언트)]
  물리 키보드/마우스                        입력 주입 (CGEvent)
  저수준 훅으로 캡처
```

## 동작 방식

- Windows에서 마우스 커서를 **Mac이 있는 쪽 화면 가장자리** 너머로 밀면 제어권이 Mac으로 넘어갑니다.
- 그 동안 Windows에서는 모든 입력이 차단(swallow)되고 TCP로 Mac에 전달됩니다.
- Mac 화면에서 반대편 가장자리(Windows 쪽)로 커서를 밀면 제어권이 다시 Windows로 돌아옵니다.
- 비상 복귀 단축키: **Ctrl+Alt+F12** (Windows에서 강제로 제어권 회수)
- 연결이 끊기면 자동으로 Windows 제어로 복귀합니다. `Ctrl+Alt+Del`은 훅으로 가로챌 수 없으므로 항상 탈출구가 됩니다.

## 빌드

양쪽 모두 Rust 툴체인이 필요합니다 (https://rustup.rs).

**Windows (서버):**

```bash
cargo build --release -p kvm-server
```

**Mac (클라이언트):** 이 저장소를 Mac에 복사한 뒤:

```bash
cargo build --release -p kvm-client
```

## 실행

**1. Windows에서 서버 실행** (Mac이 왼쪽에 있으면 `left`, 오른쪽이면 `right`):

```bash
./target/release/kvm-server.exe --mac-side left
```

처음 실행 시 Windows 방화벽 허용 창이 뜨면 **개인 네트워크 허용**을 선택하세요.

**2. Mac에서 클라이언트 실행** (`<windows-ip>`는 Windows PC의 LAN IP):

```bash
./target/release/kvm-client <windows-ip>
```

### macOS 권한 설정 (필수)

입력 주입을 위해 **손쉬운 사용(Accessibility)** 권한이 필요합니다:

1. 시스템 설정 → 개인정보 보호 및 보안 → **손쉬운 사용**
2. kvm-client를 실행하는 터미널 앱(Terminal / iTerm 등)을 추가하고 활성화

권한이 없으면 이벤트가 조용히 무시됩니다.

### 옵션

| 옵션 | 대상 | 설명 |
|------|------|------|
| `--port <n>` | 양쪽 | TCP 포트 (기본 24800) |
| `--mac-side left\|right` | 서버 | Windows 화면 기준 Mac의 위치 (기본 left) |
| `--speed <f>` | 클라이언트 | 마우스 감도 배율 (기본 1.0) |
| `--ctrl-as-cmd` | 클라이언트 | Windows Ctrl 키를 Mac Command로 매핑 (Ctrl+C → Cmd+C). 기본은 물리 매핑: Ctrl→Control, Win→Command, Alt→Option |
| `--list-monitors` | 서버 | 인식된 모니터 배치와 전환 엣지를 출력하고 종료 (진단용) |

## 다중 모니터

양쪽 모두 다중 모니터를 지원합니다:

- **Windows**: 전환 엣지는 가상 데스크톱의 가장 바깥 좌/우 가장자리입니다. 엣지를 소유한
  모니터의 실제 좌표 기준으로 세로 위치를 매핑하므로, 해상도·배치(가로/세로 회전 포함)가
  달라도 진입/복귀 위치가 어긋나지 않습니다. `--list-monitors`로 인식 상태를 확인하세요.
- **Mac**: 모든 활성 디스플레이의 배치(시스템 설정 → 디스플레이 정렬 기준)를 인식하며,
  커서가 디스플레이 사이를 자유롭게 이동합니다. 배치상 빈 공간(dead zone)으로는 커서가
  빠지지 않도록 현재 디스플레이에 클램핑됩니다. Windows로의 복귀는 전체 배치의 가장
  바깥 엣지에서만 일어납니다.
- 위치 매핑은 "엣지를 공유하는 모니터의 세로 구간"끼리 비율로 대응됩니다. 예를 들어
  Windows 왼쪽 엣지 모니터의 중간 높이에서 넘어가면 Mac 오른쪽 엣지 디스플레이의 중간
  높이로 들어갑니다.

## 개발용 도구

Mac 없이 서버를 테스트하려면 가짜 클라이언트를 사용하세요:

```bash
cargo run -p kvm-protocol --example fake_client -- 127.0.0.1 10
```

핸드셰이크 후 10초간 서버가 보내는 메시지(Enter, MouseMove, Key, ...)를 출력합니다.
이 상태에서 왼쪽 가장자리로 커서를 밀면 캡처가 시작됩니다. Ctrl+Alt+F12로 복귀하세요.

## 구조

```
crates/
├── kvm-protocol/   # 공유 메시지 타입 + 프레이밍 (u32 length prefix + bincode)
├── kvm-server/     # Windows: WH_MOUSE_LL/WH_KEYBOARD_LL 훅 캡처, 엣지 감지, TCP 서버
│   ├── hooks.rs    #   훅 콜백, 원격 모드 상태, 커서 재중심화(delta 추출)
│   └── net.rs      #   클라이언트 수락/핸드셰이크/이벤트 전달/하트비트
└── kvm-client/     # macOS: TCP 수신 → CGEvent 주입
    ├── inject.rs   #   가상 커서 추적, 버튼/모디파이어 상태, 클릭 카운트, 스크롤
    └── keymap.rs   #   Windows VK 코드 → macOS 키코드 매핑 테이블
```

### 프로토콜 요약

- 클라이언트가 접속해 `Hello` → 서버가 `HelloAck` 응답 (버전 확인)
- 서버 → 클라이언트: `Enter{edge, y_ratio}`(제어권 이동), `MouseMove{dx,dy}`, `MouseButton`, `Wheel`, `Key{vk}`, `Leave`, `Heartbeat`(2초)
- 클라이언트 → 서버: `ReturnToServer{y_ratio}` (커서가 복귀 엣지에 닿음)

## 알려진 제한 / 로드맵

- [ ] **클립보드 동기화** (2단계 목표 — 프로토콜은 확장 가능하게 설계됨)
- [ ] 한/영 전환 키(VK_HANGUL) 매핑 — 현재 Mac 쪽에서는 무시됨 (Mac 자체 입력소스 전환 사용)
- [ ] 원격 모드 중 Windows 커서가 화면 중앙에 보임 (Raw Input + 커서 숨김으로 개선 예정)
- [ ] mDNS 자동 탐색 (현재는 IP 직접 입력)
- [ ] 전송 암호화(TLS) — 현재 평문이므로 신뢰할 수 있는 LAN에서만 사용
- [x] ~~Mac 다중 모니터~~ (전체 디스플레이 배치 인식)
- [x] ~~Windows 다중 모니터~~ (모니터별 좌표 매핑, `--list-monitors` 진단)
- [ ] 트레이 아이콘 / GUI 설정
