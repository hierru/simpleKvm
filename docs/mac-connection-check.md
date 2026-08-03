# Mac 연결 실패 진단 절차 ("No route to host")

## 증상 정리

- Mac의 simpleKvm.app이 Windows 서버(192.168.1.105:24800)에 연결 시 **"No route to host"**
- 같은 Mac 터미널에서 `nc -vz 192.168.1.105 24800` → **succeeded** (이미 확인됨)
- Windows 쪽 확인 완료: GUI 서버 리슨 중, 로컬 핸드셰이크 정상, 방화벽 인바운드 허용(앱 규칙 4개 + 포트 규칙, Block 규칙 없음), PC IP = 192.168.1.105 맞음

"No route to host"(EHOSTUNREACH)는 **TCP 연결이 시작되기도 전에** 나는 오류이므로 서버 프로그램의
프로토콜/GUI 변경은 이 오류를 만들 수 없다. macOS 15+에서 **로컬 네트워크 권한이 거부된 앱**이
LAN IP로 연결할 때 정확히 이 오류가 발생한다 (팝업 없이 조용히 차단되는 경우 포함).

아래 순서대로 실행하고, 각 단계의 실제 출력과 함께 결과를 보고할 것.

---

## 0단계. 앱이 실제로 접속하는 주소 확인 (오타/옛 주소 배제)

```bash
cat ~/Library/Application\ Support/simpleKvm/client.json
```

- `server`가 정확히 `192.168.1.105`, `port`가 `24800`인지 확인.
- 다르면 그게 원인. 수정 후 재시도.

## 0.5단계. nc가 실제로 서버 프로그램까지 닿는지 눈으로 확인

`nc -vz`는 대상 IP:포트로 **실제 TCP 연결을 맺는** 명령이다 (앱이 하는 것과 같은 네트워크 동작).
이것이 새 GUI 서버까지 진짜 닿는지 양쪽에서 동시에 확인한다:

1. Windows에서 simpleKvm 서버 GUI 창의 **로그 패널**을 보이게 둔다.
2. Mac 터미널에서:

   ```bash
   nc -vz 192.168.1.105 24800
   ```

3. Mac에 `succeeded`가 뜨는 순간, Windows GUI 로그에 Mac의 IP가 찍힌
   `핸드셰이크 실패: ...` 줄이 (약 5초 내) 나타난다 — nc는 프로토콜 인사를 안 보내므로
   실패로 기록되는 게 정상이며, **Mac의 패킷이 공유기·Windows 방화벽을 통과해
   새 서버 프로그램의 accept까지 도달했다는 직접 증거**다.

## 1단계. 같은 바이너리를 터미널에서 실행 (결정적 실험 A)

서버는 그대로 두고, **실행 주체만** .app → 터미널로 바꾼다. 터미널은 이미 로컬 네트워크
권한을 갖고 있으므로, 이 실험으로 서버 원인인지 앱 권한 원인인지 갈린다.

```bash
cd simpleKvm
KVM_SHOW=1 ./target/release/kvm-client
```

(단일 인스턴스 가드가 있으므로 **먼저 simpleKvm.app을 완전히 종료**할 것. 종료 안 하면
"simpleKvm is already running"으로 그냥 꺼짐.)

- **터미널에서는 연결됨** → 서버 정상 확정. 원인은 .app의 로컬 네트워크 권한. 2단계로.
- **터미널에서도 "No route to host"** → 예상 밖. 5단계(서버 교차 실험)로 직행.

## 2단계. 수정된 Info.plist 반영 (커밋 ced9cf1)

번들 plist에 `NSLocalNetworkUsageDescription`이 없어서 권한 팝업이 아예 뜨지 않던 문제를
수정했다. 최신을 받아 재번들:

```bash
git pull
./scripts/bundle-mac.sh
plutil -p dist/simpleKvm.app/Contents/Info.plist | grep -i localnetwork
```

- 마지막 명령에서 `NSLocalNetworkUsageDescription`이 출력되어야 함. 안 나오면 pull이 안 된 것.

## 3단계. 앱 재실행 + 권한 팝업 허용

```bash
open dist/simpleKvm.app
```

- 연결 시도 시 "로컬 네트워크에서 장비를 찾고 연결하는 것을 허용하겠습니까?" 팝업 → **허용**.
- 팝업이 안 뜨면: 시스템 설정 → 개인정보 보호 및 보안 → **로컬 네트워크** → 목록에서
  simpleKvm을 찾아 수동으로 켠다.
- 목록에도 없으면 권한 DB가 꼬인 것: 앱 종료 후

  ```bash
  tccutil reset All com.simplekvm.client
  ```

  실행하고 앱을 다시 연다. (Accessibility 권한도 초기화되므로 다시 허용 필요.)
  그래도 안 되면 Mac 재부팅 후 재시도 (로컬 네트워크 권한 데몬이 재시작되어야 반영되는
  경우가 있음).

## 4단계. 성공 확인

앱에서 연결 → Windows 서버 GUI 로그에 `클라이언트 연결됨: mac (...)` 표시 확인.
커서 이동/키보드/클립보드까지 확인되면 종료. 실패 시 5단계.

## 5단계. 서버 교차 실험 (실험 B — 서버 원인 가설 직접 검증)

Windows 쪽에 **예전 CLI 서버(v1)** 바이너리가 준비되어 있다
(`C:\Workspace\simpleKvm\kvm-server-cli-old.exe`). Windows에서 GUI 서버를 중지하고
CLI 서버를 실행하도록 요청한 뒤, Mac 앱으로 다시 연결 시도:

- **여전히 "No route to host"** → 서버와 무관함이 확정 (예전 서버로도 동일 실패).
- **오류가 "handshake"/"protocol version mismatch"류로 바뀜** → TCP는 뚫렸다는 뜻이며,
  이 역시 네트워크 계층 문제가 아니었음을 의미 (v1 서버 ↔ v2 클라이언트라 버전 불일치는 정상).
- **정상 연결됨** → 서버 원인 확정. 이 결과가 나오면 Windows 쪽에서 GUI 서버의 네트워크
  계층을 재조사한다.

## 보고 양식

각 단계 번호와 실제 터미널 출력(복사)을 함께 보고:

1. client.json 내용
2. 터미널 실행 결과 (연결 성공/실패 + 오류 메시지 원문)
3. plutil 출력
4. 권한 팝업 등장 여부 / 로컬 네트워크 목록에 simpleKvm 존재 여부
5. (필요시) 실험 B 결과
