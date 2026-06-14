<h1>Sego <img src="assets/sego-ui-icon.png" width="34" height="34" alt="Sego 鍥炬爣" align="right"></h1>

<p align="center">
  <strong>AI Coding 鐨勫伐绋嬩俊浠昏繍琛屾椂</strong><br>
  璁?AI 鐢熸垚鐨勪唬鐮佸彉寰楀彲瀹℃煡銆佸彲楠岃瘉銆佸彲澶嶇洏銆佸彲浜や粯銆?</p>

<p align="center">
  <a href="#蹇€熷紑濮?><img src="https://img.shields.io/badge/蹇€熷紑濮?5鍒嗛挓-blue?style=flat-square" alt="蹇€熷紑濮?></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/璁稿彲璇?MIT-green?style=flat-square" alt="MIT 璁稿彲璇?></a>
  <img src="https://img.shields.io/badge/Rust-鍘熺敓-orange?style=flat-square" alt="Rust 鍘熺敓">
  <img src="https://img.shields.io/badge/骞冲彴-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey?style=flat-square" alt="鏀寔骞冲彴">
</p>

---

## Sego 鏄粈涔?
Sego 鏄潰鍚?AI Coding 鏃朵唬鐨勫伐绋嬩俊浠诲伐鍏枫€傚畠涓嶆槸鍐嶅仛涓€涓€滄浛浣犲啓鏇村浠ｇ爜鈥濈殑 Agent锛屼篃涓嶆槸浼犵粺 IDE锛涘畠鏇村儚涓€灞傚紑鍙戝伐浣滄祦鐨勬湰鍦颁俊浠昏繍琛屾椂锛屽府鍔╁紑鍙戣€呭垽鏂?AI 鍐欏嚭鐨勪唬鐮佹槸鍚﹀€煎緱杩涘叆鎻愪氦銆佸悎骞跺拰浜や粯銆?
浣犲彲浠ョ户缁娇鐢?Claude Code銆丆odex銆丆ursor銆丱penHands 鎴栧叾浠?AI 缂栫爜宸ュ叿鐢熸垚浠ｇ爜銆係ego 璐熻矗鍦ㄤ唬鐮佽繘鍏?Git銆丳R 鎴栦氦浠樹箣鍓嶏紝琛ヤ笂宸ョ▼鍖栫殑瀹℃煡銆侀獙璇併€佽褰曞拰浜ゆ帴璇佹嵁銆?
涓€鍙ヨ瘽锛?
```text
AI 璐熻矗鐢熸垚锛孲ego 璐熻矗璇佹槑瀹冨€煎緱琚悎骞躲€?```

---

## 涓轰粈涔堥渶瑕?Sego

AI 缂栫爜宸ュ叿宸茬粡鑳藉揩閫熺敓鎴愪唬鐮侊紝浣嗙湡瀹炲紑鍙戜腑鐨勯闄╁線寰€鍑虹幇鍦ㄥ悗鍗婃锛?
- 淇畬涓€涓?bug 鍚庯紝鐩稿叧鏁版嵁閾捐矾銆佽皟鐢ㄩ摼銆佹祴璇曞拰鏂囨。娌℃湁鍚屾瀵归綈銆?- AI 鍙叧娉ㄥ綋鍓?diff锛屽鏄撳拷鐣ュ巻鍙蹭笂涓嬫枃銆侀」鐩害鏉熷拰宸叉帴鍙楄鍒欍€?- 浠ｇ爜 review 鍙粰鑷劧璇█寤鸿锛岀己灏戦闄╃瓑绾с€佽瘉鎹€佺姸鎬佸拰鍙拷韪褰曘€?- 娴嬭瘯閫氳繃涓庡惁缂哄皯缁撴瀯鍖栨矇娣€锛屼笅涓€娆′粛鐒朵粠澶村垽鏂€?- 鍗曟瀵硅瘽瓒婃潵瓒婇暱锛岄」鐩粡楠屾棤娉曠ǔ瀹氬鐢紝涔熷緢闅句氦鎺ョ粰鍙︿竴涓?Agent 鎴栧紑鍙戣€呫€?
Sego 甯屾湜鎶婅繖浜涢棶棰樻敹鏁涙垚涓€涓湰鍦颁紭鍏堛€佸彲瀹¤銆佸彲鎸佺画婕旇繘鐨勫伐绋嬮棴鐜細

```text
鐢熸垚浠ｇ爜 -> 瀹℃煡鏀瑰姩 -> 楠岃瘉璇佹嵁 -> 娌夋穩涓婁笅鏂?-> 杈呭姪浜や粯
```

---

## 褰撳墠 MVP 鑳藉仛浠€涔?
褰撳墠寮€婧愮増鏈粛澶勫湪 MVP 杩唬闃舵锛岄噸鐐规槸鎶?review銆乿erify 鍜屾湰鍦板伐绋嬭蹇嗚窇閫氥€傚凡鍏紑鐨勮兘鍔涗富瑕佸洿缁曗€滄彁浜ゅ墠宸ョ▼淇′换闂幆鈥濆睍寮€銆?
### 鎻愪氦鍓?readiness gate

```bash
sego /review ready
```

`/review ready` 浼氱粰鍑?staged 鎻愪氦鍓嶇姸鎬侊細

- 褰撳墠鏆傚瓨鍖烘枃浠舵暟閲忋€?- staged safety lock 缁撴灉銆?- `/verify fast` 鐨勮鍒掑懡浠ゃ€?- 涓嬩竴姝ュ缓璁細鏄惁闇€瑕佸厛琛?staged銆佷慨澶嶅畨鍏ㄩ闄┿€佹墽琛?review 鎴?verify銆?
瀹冩槸鍙鎶ュ憡锛屼笉浼氳皟鐢ㄦā鍨嬨€佷笉浼氭墽琛?build/test銆佷笉浼氬畨瑁呬緷璧栥€佷笉浼氫慨鏀规枃浠躲€?
### staged 瀹夊叏閿?
```bash
sego /review safety staged
```

鍙壂鎻?Git 鏆傚瓨鍖烘枃浠讹紝鐢ㄤ簬鎻愪氦鍓嶅彂鐜版槑鏄剧殑鍒濈骇瀹夊叏椋庨櫓锛屼緥濡傜枒浼煎瘑閽ユ枃浠躲€佺‖缂栫爜瀵嗛挜銆佸嵄闄?shell 鍛戒护銆佹湰鏈虹粷瀵硅矾寰勭瓑銆?
瀹冮€傚悎浣滀负 vibe coding 鏂版墜鐨勭涓€灞傚畨鍏ㄩ攣锛屼絾杩欏彧鏄?Sego 鐨勪竴涓簲鐢ㄥ満鏅紝涓嶆槸 Sego 鐨勫畬鏁翠骇鍝佽竟鐣屻€?
### 浠ｇ爜瀹℃煡

```bash
sego /review staged
sego /review
```

Sego 鍙互閽堝 staged diff 鎴栧綋鍓嶅伐浣滃尯鏀瑰姩鍙戣捣鍙浠ｇ爜瀹℃煡锛岃緭鍑虹粨鏋勫寲 findings锛屽苟灏介噺淇濈暀闂浣嶇疆銆佷弗閲嶇▼搴︺€佽瘉鎹€侀闄╄鏄庡拰淇寤鸿銆?
### 瀹℃煡鍘嗗彶涓庣姸鎬?
```bash
sego /review list
sego /review show <review-id>
sego /review status <review-id>
sego /review mark <review-id> <finding-id> <status> [note]
```

瀹℃煡缁撴灉浼氭矇娣€涓烘湰鍦拌褰曪紝渚夸簬鍥炵湅銆佹爣璁般€佸鐩樺拰浜ゆ帴銆俧inding 鐘舵€佹敮鎸佸悗缁拷韪紝涓嶅繀鎶婃瘡娆?review 閮藉綋鎴愪竴娆℃€ц亰澶╃粨鏋溿€?
### 楠岃瘉璁″垝

```bash
sego /verify fast
sego /verify
```

Sego 浼氭牴鎹綋鍓嶉」鐩瘑鍒熀纭€楠岃瘉璁″垝锛屼緥濡?Rust 椤圭洰鐨?`cargo build` / `cargo test`锛屾垨 Node 椤圭洰鐨?test/build 鑴氭湰銆俙/review ready` 鍙睍绀洪獙璇佽鍒掞紱鐪熸鎵ц浠嶇敱 `/verify` 鏄庣‘瑙﹀彂銆?
### 宸ュ叿閾惧缓璁?
```bash
sego /review tools
```

鏈湴鍙鎺㈡祴椤圭洰璇█鍜屽伐鍏烽摼锛岀粰鍑哄缓璁墽琛岀殑妫€鏌ュ懡浠ゃ€傚畠涓嶄細瀹夎渚濊禆锛屼篃涓嶄細鑷姩鎵ц澶栭儴宸ュ叿銆?
---

## 鎺ㄨ崘宸ヤ綔娴?
濡傛灉浣犲凡缁忎娇鐢?AI 宸ュ叿瀹屾垚浜嗕竴杞唬鐮佷慨鏀癸紝鍙互鎸変笅闈㈢殑椤哄簭鎻愪氦鍓嶈嚜鏌ワ細

```bash
git add <files>
sego /review ready
sego /review safety staged
sego /review staged
sego /verify fast
```

鎺ㄨ崘鐞嗚В鏂瑰紡锛?
| 姝ラ | 浣滅敤 |
|---|---|
| `git add <files>` | 鏄庣‘鏈鍑嗗鎻愪氦鐨勮寖鍥?|
| `/review ready` | 鏌ョ湅鎻愪氦鍓嶇姸鎬佸拰涓嬩竴姝ュ缓璁?|
| `/review safety staged` | 鍏堢敤鏈湴瀹夊叏閿佹帓闄ゆ槑鏄鹃闄?|
| `/review staged` | 瀵规殏瀛樺尯 diff 鍋氭ā鍨嬭緟鍔╁鏌?|
| `/verify fast` | 鏄庣‘鎵ц蹇€熼獙璇侊紝鐢熸垚鍙氦浠樿瘉鎹?|

---

## 浜у搧鏂瑰悜

Sego 鐨勭洰鏍囦笉鏄垚涓轰竴涓€滃彧鏈?review 浼樺娍銆佹病鏈夌紪鐮佷紭鍔库€濈殑瀛ょ珛宸ュ叿銆傚畠鐨勪骇鍝佸畾浣嶆槸 AI Coding 宸ヤ綔娴佷腑鐨勫伐绋嬩俊浠诲眰锛?
```text
AI 缂栫爜宸ュ叿        ->  璐熻矗鐢熸垚浠ｇ爜
Sego              ->  璐熻矗瀹℃煡銆侀獙璇併€佽褰曘€佷氦鎺ヨ瘉鎹?Git / CI / 骞冲彴    ->  璐熻矗鍚堝苟銆侀儴缃插拰鐢熶骇浜や粯
```

鏈潵 Sego 浼氬洿缁曞洓涓柟鍚戞紨杩涳細

| 鏂瑰悜 | 璇存槑 |
|---|---|
| Review | 璁╀唬鐮佸鏌ユ洿缁撴瀯鍖栵紝杈撳嚭椋庨櫓绛夌骇銆佽瘉鎹€佹枃浠朵綅缃拰寤鸿 |
| Verify | 灏嗗鏌ョ粨璁轰笌鏋勫缓銆佹祴璇曘€乴int 绛夐獙璇佽瘉鎹繛鎺ヨ捣鏉?|
| Memory | 娌夋穩椤圭洰涓婁笅鏂囥€佸巻鍙茶鎶ャ€佸凡鎺ュ彈瑙勫垯鍜岄獙璇佽褰?|
| Ship | 鐢熸垚闈㈠悜 commit銆丳R 鍜屽彂甯冪殑浜や粯鎶ュ憡 |

---

## 閫傚悎璋佷娇鐢?
### 涓汉寮€鍙戣€?
濡傛灉浣犲凡缁忓湪浣跨敤 AI 鍐欎唬鐮侊紝Sego 鍙互甯綘鍦ㄦ彁浜ゅ墠澶氫竴灞傚伐绋嬪寲纭锛氱湅娓呮敼鍔ㄣ€佸彂鐜伴闄┿€佽ˉ瓒抽獙璇佽瘉鎹€?
### vibe coding 鏂版墜

濡傛灉浣犱笉鐔熸倝浠ｇ爜瑙勮寖銆丟it 鎻愪氦杈圭晫銆佸畨鍏ㄩ闄╁拰楠岃瘉娴佺▼锛孲ego 鍙互浣滀负鎻愪氦鍓嶅畨鍏ㄩ攣锛屾彁閱掍綘涓嶈鎶婃槑鏄惧嵄闄╃殑鍐呭鐩存帴鎻愪氦銆?
### 寮€婧愰」鐩淮鎶よ€?
Sego 鍙互浣滀负鏈湴 review 鍜?verify 杈呭姪宸ュ叿锛屽湪 PR 鍓嶆彁鍓嶆毚闇查棶棰橈紝闄嶄綆缁存姢鑰?review 鎴愭湰銆?
### AI Coding 宸ュ叿閲嶅害鐢ㄦ埛

濡傛灉浣犵粡甯稿湪澶氫釜 AI 宸ュ叿涔嬮棿鍒囨崲锛孲ego 鍙互浣滀负缁熶竴鐨勫伐绋嬩俊浠诲眰锛屽府鍔╀綘鎶婁笉鍚屽伐鍏风敓鎴愮殑鏀瑰姩绾冲叆鍚屼竴濂楀鏌ュ拰楠岃瘉娴佺▼銆?
---

## 蹇€熷紑濮?
### Windows

```powershell
irm https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.ps1 | iex
set ANTHROPIC_API_KEY=sk-your-key
sego
```

### macOS / Linux

```bash
curl -fsSL https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.sh | bash
export ANTHROPIC_API_KEY="sk-your-key"
sego
```

### 浠庢簮鐮佹瀯寤?
```bash
git clone https://github.com/007M7/Sego-Agent.git
cd Sego-Agent/rust
cargo build --release
./target/release/sego
```

---

## 甯哥敤鍛戒护

```bash
sego
```

杩涘叆浜や簰寮?CLI銆?
```bash
sego "鎬荤粨褰撳墠椤圭洰缁撴瀯"
```

鎵ц涓€娆℃€т换鍔°€?
```bash
sego /review ready
```

鏌ョ湅鎻愪氦鍓?readiness gate銆?
```bash
sego /review staged
```

瀹℃煡鏆傚瓨鍖烘敼鍔ㄣ€?
```bash
sego /review list
```

鏌ョ湅鏈湴瀹℃煡鍘嗗彶銆?
```bash
sego /verify fast
```

鎵ц蹇€熼獙璇併€?
```bash
sego status
```

鏌ョ湅褰撳墠宸ヤ綔鍖虹姸鎬併€?
```bash
sego --resume latest
```

鎭㈠鏈€杩戜竴娆′細璇濄€?
---

## 褰撳墠鐘舵€?
Sego 褰撳墠澶勪簬鏃╂湡 MVP 闃舵锛岄噸鐐规槸鎶?review銆乿erify 鍜屾湰鍦板伐绋嬭蹇嗚窇閫氥€傞儴鍒嗚兘鍔涗粛鍦ㄥ揩閫熻凯浠ｄ腑锛屾帴鍙ｅ拰杈撳嚭鏍煎紡鍙兘缁х画璋冩暣銆?
宸插畬鎴愮殑鍏紑鏂瑰悜锛?
- CLI 鍩虹鑳藉姏銆?- 鏈湴浼氳瘽鎭㈠銆?- `/review` 鍒濈増銆?- staged code review銆?- 缁撴瀯鍖?review findings銆?- review 鍘嗗彶鏌ョ湅銆?- finding 鐘舵€佹爣璁般€?- staged safety lock銆?- `/review ready` 鎻愪氦鍓?readiness gate銆?- `/verify` 鍒濈増銆?- 鍩虹鏉冮檺杈圭晫銆?- GitHub Actions 鍩虹楠岃瘉銆?
姝ｅ湪鎺ㄨ繘锛?
- 鏇寸ǔ瀹氱殑瀹℃煡杈撳嚭缁撴瀯銆?- 瀹℃煡鎶ュ憡涓庨獙璇佽瘉鎹叧鑱斻€?- 鏇存竻鏅扮殑 PR / commit 鎶ュ憡銆?- 闈㈠悜涓汉寮€鍙戣€呯殑瀹屾暣 MVP 浣撻獙銆?- 鍙鐢ㄧ殑宸ョ▼涓婁笅鏂囪蹇嗐€?
---

## 寮€婧愯竟鐣?
褰撳墠 README 鍙睍绀?Sego 鐨勫叕寮€浜у搧瀹氫綅鍜?MVP 浣跨敤鏂瑰紡銆傛洿瀹屾暣鐨勫唴閮ㄥ紑鍙戞祦绋嬨€佸伐绋嬭蹇嗗疄鐜般€佸崗浣滃崗璁€佽瘎瀹＄瓥鐣ャ€侀獙璇侀摼璺拰鍟嗕笟鍖栬璁℃殏涓嶅湪寮€婧愭枃妗ｄ腑灞曞紑銆?
闅忕潃 MVP 绋冲畾锛岄」鐩細閫愭寮€鏀炬洿澶氬彲浠ュ叕寮€鐨勮兘鍔涜鏄庡拰浣跨敤绀轰緥銆?
---

## 璁稿彲璇?
Sego 浣跨敤 MIT License銆?
---

## 涓€鍙ヨ瘽鎬荤粨

```text
Sego 涓嶆槸鏇夸綘鍐欐洿澶氫唬鐮佺殑宸ュ叿銆?Sego 鏄府浣犲垽鏂?AI 鍐欏嚭鐨勪唬鐮佹槸鍚﹀€煎緱鍚堝苟鐨勫伐绋嬩俊浠昏繍琛屾椂銆?```
