# simpleKvm 프로젝트 작업 규칙

이 파일은 저장소를 여는 모든 Claude Code 세션(Windows·Mac 모두)에 자동 적용된다.

## 언어: 무조건 한글
- 답변, 설명, 보고, 질문은 **항상 한글**로 쓴다.
- 도구 실행 시 사용자에게 보이는 설명(description)도 **한글**로 쓴다.
- git 커밋 메시지, PR 제목/본문, README·CHANGELOG 등 문서, 코드 주석도 **한글**로 쓴다.
- 식별자·명령어·경로·API 이름·오류 메시지 원문 같은 기술 용어만 원문 유지.

## 보고 방식
- 검증·테스트 결과는 "통과/정상/완료"라는 말만으로 끝내지 않는다.
  **항목 / 절차 / 통과 기준 / 실측값 / 판정**을 항목별(표)로 보고한다.
- 검증하지 않은 항목은 "미검증"으로 명시한다.
- 사용자가 실행 중인 프로그램·창·프로세스를 조작(이동·복원·종료·설정 변경)하기 전에
  먼저 보고하고 허락을 받는다. 읽기 전용 조사는 바로 해도 되지만 결과를 즉시 보고한다.

## 프로젝트 구조 요약
- `crates/kvm-protocol` 공유 메시지·프레이밍, `crates/kvm-server` Windows 서버(GUI, 훅, 트레이),
  `crates/kvm-client` macOS 클라이언트(GUI, CGEvent 주입).
- Windows 세션은 서버, Mac 세션은 클라이언트를 담당하며 GitHub `main`으로 동기화한다.
- 서버 진단 로그: `%APPDATA%\simpleKvm\server.log`. 자세한 내용은 README.md, CHANGELOG.md 참고.
