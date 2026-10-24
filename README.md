# FilmCraft GM컷

FilmCraft 0.10.0 안에서 무음 후보를 검토하고 컷을 적용하는 GM컷 수정판입니다.
공식 FilmCraft 배포판이 아니며, [storytold/filmcraft](https://github.com/storytold/filmcraft) `v0.10.0`을 기반으로 합니다.
독립 브라우저 도구는 [Glory-yk/gmcut](https://github.com/Glory-yk/gmcut)에 있습니다.

An independent FilmCraft 0.10.0 fork with a native GMcut silence-review workspace. Source distribution; the optional local Python analyzer is separate from FilmCraft's transcription backend.

## 사용할 수 있는 기능

- **창 → 작업영역 → GM컷**에서 파일을 가져오고 무음 후보를 분석합니다.
- 새 파일 기본값: 최소 무음 **0.05초**, 감도 **보통**, 자동 유지 **0초**.
- **1 / 2 / 3**: 기본 전체삭제 / 0.2초 / 0.3초. 각 키의 유지 시간을 바꿀 수 있습니다.
- 선택 즉시 미리보기와 다음 후보 이동. **P** 재생, **← / →** 이동, **S** 건너뛰기, **⌘Z** 검토 실행취소.
- 자동 정리 또는 수동 검토 후 **컷 적용**을 누르면 현재 작업 시퀀스에 한 번의 편집으로 반영합니다. 적용 후 일반 실행취소가 가능합니다.
- 원본 파일을 덮어쓰지 않으며, 기존 FilmCraft 편집·ASR·자막 명령을 유지합니다. 기존 자막은 같은 리플 편집 경로로 이동합니다.
- 분석·검토·컷 적용만으로 영상 출력은 시작되지 않습니다.

0초는 검출된 무음 안에 완전히 들어가는 영상 프레임을 제거합니다. 프레임보다 짧은 경계는 남을 수 있습니다. 0.2초·0.3초는 남기는 무음의 총 길이이며, 발화 보호 여백 때문에 짧은 후보는 보존될 수 있습니다. 검출 오류는 가능하므로 말끝과 작은 목소리를 미리보기로 확인하세요.

## macOS에서 소스로 실행

현재 검증 대상은 Apple Silicon macOS입니다. Git, Rust **1.95.0**, Xcode Command Line Tools, PyTorch를 지원하는 Python(권장 3.11)이 필요합니다. Python 패키지와 모델은 아래 명령을 직접 실행할 때 설치됩니다. 네이티브 분석에는 FFmpeg가 필요하지 않습니다.

```sh
git clone https://github.com/Glory-yk/filmcraft-gmcut.git
cd filmcraft-gmcut
python3.11 -m venv .venv
.venv/bin/python -m pip install -r apps/filmcraft/gmcut_analysis/requirements.txt
export FILMCRAFT_GMCUT_MODEL="$HOME/.cache/gmcut/silero_vad.jit"
.venv/bin/python apps/filmcraft/gmcut_analysis/setup_model.py --output "$FILMCRAFT_GMCUT_MODEL"

git clone https://github.com/storytold/craft-fonts.git ../craft-fonts
git -C ../craft-fonts checkout abb83316d96aa59c1cf64784289e378fe9fa5695
export CRAFT_FONTS_DIR="$(cd ../craft-fonts && pwd)"
export CRAFT_FONTS_REQUIRED=1
packaging/gmcut/run-native.sh --data-dir "$PWD/.gmcut-data/filmcraft"
```

모델 준비 도구는 고정된 공식 Silero 모델을 내려받아 크기와 SHA-256을 검사하고 사용자 캐시에 저장합니다. 이미 준비한 모델을 사용하려면 `FILMCRAFT_GMCUT_MODEL`에 절대 경로를 지정하세요. Python 경로는 `FILMCRAFT_GMCUT_PYTHON`으로 지정할 수 있습니다. 구체적인 실행·패키지 옵션은 [패키징 안내](packaging/gmcut/README.md)를 참고하세요.

위 실행 예시는 별도 데이터 폴더를 사용합니다. `--data-dir` 없이 실행하면 기존 FilmCraft 데이터 폴더를 사용할 수 있으므로 여러 수정판을 동시에 실행하지 마세요.

처음 빌드는 오래 걸릴 수 있습니다. GM컷 분석 환경이 없더라도 기존 FilmCraft 기능은 열리며, GM컷 분석에서 환경 오류를 안내합니다. 일반 전사 백엔드를 GM컷으로 교체하지 않습니다.

## 로컬 앱 묶음 만들기

```sh
cargo build --locked --release -p filmcraft
.venv/bin/python packaging/gmcut/build-macos.py --help
```

전용 도구는 새 `dist/FilmCraft GMcut.app`만 만들며, `/Applications`의 앱을 설치·덮어쓰기·종료하지 않습니다. 이 앱은 선택한 Python 환경을 참조하므로 다른 Mac에는 분석 환경을 별도로 준비해야 합니다. 서명·공증된 다운로드용 설치 파일은 아직 제공하지 않습니다. 기존 프로젝트와 기존 앱을 먼저 백업한 뒤 테스트 복사본에서 사용하세요. 공식 업데이트는 사용자 수정판을 덮어쓸 수 있습니다.

## 현재 범위와 검증

GM컷은 시작점 0·속도 1의 단일 오디오 또는 연결된 A/V 클립을 지원합니다. 여러 클립·겹침·트림·속도 변경이 있는 시퀀스는 지원하지 않습니다. 분석은 최대 1시간이며 자막과 ASR 단어가 있는 경우 손상을 막는 검사를 수행합니다. 키 설정은 현재 앱 실행 동안 유지되며, 재실행 후의 저장은 아직 구현하지 않았습니다.

선별 회귀 검사: GM컷 엔진 10개, UI 2개, 기존 전사 8개, 자막 1개, PCM 어댑터 3개. 공개 준비 과정의 검사 결과와 수정 범위는 [GMcut 개발 기록](docs/GMcut.md)에 적습니다. 공개 준비 최종 전체 검사도 통과했습니다: 테스트 3,533개 성공·실패 0개·기본 제외 45개, 코드 형식·경고·구조·61개 자산 출처·49개 크레이트 웹 빌드 확인. 검증용 빌드 설정은 개발 기록에 적었습니다.

```sh
cargo fmt --check
cargo test --locked -p filmcraft-engine gmcut
cargo test --locked -p filmcraft-ui-egui --test gmcut_ui
cargo test --locked -p filmcraft gmcut_analyzer
.venv/bin/python -m unittest discover -s packaging/gmcut -p 'test_*.py'
cargo xtask ci
```

## 라이선스·출처

기반 커밋: `cdf43fd366e378da266cb6d3c94549a45244be1f` ([상위 프로젝트](https://github.com/storytold/filmcraft/tree/cdf43fd366e378da266cb6d3c94549a45244be1f)). 기존 코드·저작권·기여 이력은 보존합니다. Rust 통합 변경은 **MIT OR Apache-2.0**, 별도 Python 분석기는 **MIT**입니다. [NOTICE](NOTICE), [ATTRIBUTION](ATTRIBUTION.md), [LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)를 참고하세요.

상위 프로젝트의 제한된 ArtCraft 로고·워드마크는 이 수정판에서 제외했습니다. 공개 저장소에는 개인 영상·프로젝트·앱 백업·로컬 로그·모델 가중치를 넣지 않습니다. Silero와 폰트의 별도 라이선스는 준비 도구와 앱 패키지에 보존합니다.
