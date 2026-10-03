#!/usr/bin/env python3
"""Captura sanitizada Claude→Codex; resposta simulada, sem conta ou inferência real.

Execução futura, autorizada, a partir da raiz do checkout (dependências já instaladas):
  uv run --project backend --no-sync python scripts/probe-claude-to-codex.py \
    --binary /caminho/absoluto/releases/0.159.3-.../bin/codex \
    --output-dir /tmp/claude-to-codex/native-proof-01

A saída deve ser NOVA: não reutiliza nem apaga diretório existente. Tudo fica privado,
incluindo fonte artificial, catálogo, configuração, pedidos HTTP e diagnóstico.
O modelo fixo identifica esta captura, não uma preferência do recurso de produção.
O roteiro usa convert_snapshot/prepare_import reais; apenas o storage _base é privado.
Somente stdio e HTTP SSE em 127.0.0.1. Não prova compreensão, inferência, aceitação
multimodal pelo servidor, restauração física, TUI, interface do Hangar ou Windows.
"""
from __future__ import annotations

import argparse
import asyncio
import base64
from dataclasses import asdict
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import logging
import os
from pathlib import Path
import signal
import struct
import sys
import threading
import traceback
import uuid
import zlib


SOURCE_COMMIT = "01fc69f4026735edfdf6789820549727a4867b11"
SOURCE_URL = f"https://raw.githubusercontent.com/openai/codex/{SOURCE_COMMIT}/codex-rs/models-manager/models.json"
SOURCE_CATALOG_SHA256 = "fd219bd9f061278275f528939f82f54d2eb97df4b25c23b022adbe48813d920b"
SOURCE_MODEL_SHA256 = "15926988a7380812a64af708f8c7b5ea0235797b2707351b3ae05d3fc3fd9da2"
MODEL = "gpt-5.6-luna"
VERSION = "0.159.3"
# Registro completo da fonte primária fixa, comprimido para preservar as instruções.
MODEL_DATA = (
    "c-rlK>y9JWk>;x;$_Au1$ZAXWcxPK00c}~5dq$Ur)LM_>HpnEItV$J`$;nJsH7#i2huA+0?7p0zWaInda!w{$EzRuAVi(u}#;PWh"
    "C(kAB@kN~Puln8b=IU8-b#oZze|P<x`R>@1SF_^k&{f;2TRc>&zFpr{LoW{v-LaCd`s3lycEjSn>Q-%E52y6irrMUr-T14kUHgC$"
    "%fn%JS{%w@eY+UicDEQ#hYH_syQ<o@-5w9g`xbpwcGG8u>dmkyo6Vx$mp2ujteeAeSnS(PxvPh|!u<Zk$M_|m`g59WuRmODs-diR"
    "i?*w8>ZaVKnah8gbve{+vpBT7dX1TXe<j1~>)X4k=`r%I-q(YC^xiM!|9;3{a45TSx2twK@wNP&?HNDw(bn=`{$tTLyAwv;%cj)}"
    "nP9`c>tz~v@czB&mt9r%ZBsWli)t%-hM6?S-A<l5_LY3wAKIp``i1OCb#`L?+0Pfp!=@a#dE0W=SKQb2?kH31?`m1W;@D%`>ZX^W"
    "*4Um)&&tt0@}y0`A3O-P)y-QUy*ZZMrY@VbvIaZVRfpZ8JPxh=bzfHx@}GY>)?Ky9rjs7r)P1?yjlQ(!`$aEgv})f}8yhKmCLbT?"
    "BZMNS(bYUQn~i?nAF6d-?nvPF#ky@sr4MzpX))^i|LJ|95&WclvpD<q@BLf8blvU`<$73<&=yW)G4lSfxGnoz(8RCgS63OUr6cW+"
    "`+eDw<(jr3lgK_CcI9c&lzUR%XD?ro%Re_R-&Fm&s}F<G+gH0y)yZ!3Lm`_~2#wV1TF9>8qOUK%c30h3V5F;kwW*KfVxu>1@#ryf"
    ">GxMbyDl3h^Y|1q2x2ZC>fyH7)i<|;%q~W}67I`~O0S=c_S<q-HftG24z1d7Wid#xsSYx@j8Vw^y3?jS6+_wI_0!RB<@FcF{!B(1"
    "%_<uZ$F{596o;-ATHK!v`R0)^KY25BWq~jMO~VScT^RI>@=d)z?k|ow+*ZY{a9t(4o<^ngysdUg2<yM({d?I%yyL#^>s7sTIbZh_"
    "zWcfnVJi>r>WMu2<@I~N{`K`Q$0Q>1pghz`0F?J4p-UZh(H!4oBJOb0mE=X0%sW?HNp0<Gd4y72Sm91MO&4EPLjl_VO##Y(-MntT"
    "I(EelqHo|wMJ?n}-!!#QeAx`UlaPE@B@UPseOsJb`4MGTK@RPUn-*l#6x(Abo7i0!UsvT?H?<!4pRzd>-?b~*<8mlAZPhpLfV3%#"
    "ift!Sp%(I~ML6t)!YiTE>(|W{<<%gM?6HHT%Yq-Mm}}8q-0}Y$;%Hy>m{!lN$>r)|sDxTdDub&(w#TBB*M2O&c{7u#6;vw4s_ca?"
    "+NMzPQ1Ch763~UHm9FS-@iUPr#Y5ZO^&+XN8K2;eV0p5ALo1}Ylap)Z1S(}(8DU=>WsN)hS$DEw<qi+adyjQf$v)r8I3lU9aVS44"
    "UJ9|a4OIFGA3X1UvL~CnOuBlrmd(P%L5jylPHd2!<_pVWeUuG4_Oqg^<s=SiJYClw_6JusU_DsGu@R})5ArA2qTR;2XY%wNJ`>>$"
    "DkfnSTaj6Xa8bQG&4do_D*i*Bl0$4U0i3i>rYYLpMow8MNEe71+?MwhWQ@?>tWYxx$KGc<Dr8$ch$ND^^i_qiF(uji-R8RZ!>tfp"
    "91S+U+;NrzDRhSKPI6@J5$~?5ozfmZ#SYdD)-#B<@maQ@-J(CPzN>_4?q#n&5b}LT1TTIldr+m9@FzG7jx5q-XM0%x%dz4bKh8F*"
    "PtVJv8{ID179mlH^X*aCx@x{_Ptex7bMovv8{H&7(V(gtJ0}a1pB7?n4GmZhdwO#e9nB4qncjAILZ-AW_aa5gP9-{rYmbA_&jIVy"
    "UEO2b${l8RD()INS}qzN_IKc5XNN%q!2yH~q7yQZ<@Q{n9AYa+UtnmgPiXp34_nxO+?rlw&w3{|t-V+`Mu$xVoES#WRd>o)D;t$<"
    "1H{T;3?(*5J&0*FbB;af$?*5s_e9-XuJYN>ia&@9!4~wxX;+ce?%P^~fE?W%#&|jK>xmMsc9clH4!;6*i+mB&q?T>Cl|7VKfhqdI"
    "$ay7FOV-aFk~M$ZS6m`hglMDeO=3fKtq3=4h3r14j*t2bY!6L)Z$L0y%Zn|SDL=jk4UAq`w4F`{L{HB0(Pr3Xj{h-_^9#`y1$RmG"
    ";7n)$!oy8QIfs>)5O*?`h>R7Sm5PELV;Ar0D<MHUk<$B>@bl)0Nt+`~sh#K!vw){V4G)!Yb7_0W)nQ;-i_M^%yOo1)A7l-pg>)F~"
    "7uR)Nyt)+$kPX>a@Ixr5<O?^F&2Z>Cp+ET$Bq3usExfAMw;;k@MvpQ8zWp3r8<Z&|E>Fmrl$01UJC#Ht%MLw{LHwLM9N$ev0`01E"
    "I(Pd;*0>fEglAJ`r@L-%s~(&#Rz?FM%nwC`uc%jej1NY=@+RjP5m#j|Ca^4aRj%)DL<q?5z|o>zL}SnAMNb>jb-S6|rlrZ8o!Y%}"
    "M)hC@z~-o%A)A8-ds#m8MpHdN_PpxKrWf_9N<!B)w1;}#t2WS8jdm6P5|R>~apH&+*%NI`(cQoxHEq$V_~hD6*B8S>y{<y5s3MU8"
    "Fcs1GvZ+E8;!MetVCv$#vb$*;J&sU1#SllYp0TiuxTL<Xo9ae<bEQJr5!tPyh&9pK)$Y(IvdGn==_k+vBn1`>-&e$e8l<br1S}*K"
    "#;Le0F^xu4wQbkB6#PEs2segIZId;l;SXYzgl_U}9IM-h7TQJVNmj^n9LkgG?6Gnm4e{)CGv|J({bRl|euF2LI96GrxLivg{MvtU"
    "s;iw6Vosy#Rk@ZQj$Ko%L{(R6d6dN#F^P1*`okqZ&0q1ldF2D{OZ~n4SV%H{D)MVrh0P>-YPX~52Br@K;v}0c>i53f9jSkAE8G9-"
    "<cbV-sR)aWQlOC4T7_o0rY?qffdvf?+VG)zxGqE@=f86o8c*36LJT77br%lsx5Z%N{J23pdmj8P8O>kAkz*@;siHT8$7p`q98@vV"
    "PKFU6szw}_O-$CTEF}!FeT;D-z%jV~&K@?qbF&dGAY!&P;ut*>%!{eYqqPzJO{Fe@O&StI9*$|lA~8xFjwEU~#N4u&g;L|9NqzpZ"
    "2H)LqRzb3`a!4J)ffCcASt53%#UEy83=1`Z5QOMq@R%IB#u4hjnA)I#S{-Sb$;*V2#3mJ`w(~9StJn)sNY5#R`I;Tv`21_UOYHLw"
    "stpT?v-h7Hxt6<%Z<ODv<-@r{*})#IMCKMly{}9}$|`#-b7s~Og*3ibm82{J&1(S?Sf2yLshRrnirsiiM5l_8bqZFIx88!zVC<0~"
    "bd9D#@QZfMb!{@v69rKvru$t>T9XqKfib}I+ZQ4a;f;L2$Faz|Im)7h9y%Tbwup9+I_<QP`Raq()?y7Fv5}|Y7V=|X_Y|2Nn`&9t"
    "o4tI~2DQuh!5U;eWlsrbggGpprEob`l|-FO!4Je;=d&ut#3yVT*Mhny(8u%{ckPWD7q`$#vgvS{hu*D^7jn2Af;Vnl>x<0&<4(%-"
    "bmJ-N+}RBQc}WlptyE&+#j_x+Te0qStIyq3rPJ*6uqAVHCuMWK?WA0ACcd%#ID4UK@fEpkUH8hmVveovz+)0+;DPdYMQg5CWi*)&"
    "q^>pTKy4q)EX;?Px;&Ga`uKl2x_WzZBRR8^oEyWw1>dVVMNO?FYTlO%thZ^sBJ>fgctl2_CuQB@7YvM^Z28WS%MP~L9ASqhb*Pq*"
    "8$d#`vVFU1JB066o|7WCfyNpwhu{2Cs6<}9>3#IJxoK%0$i{IugdM*8>Q!;zc&c;5%XqiC7H)-=E@=<^>ZurUB{mZOaMAq4w<DX4"
    "ZH9`AbowHcg@NpGeOYYV7a1k1^ZNL(8iCpAsl)k|@Us)&EIyERNOV>{fG^TE^H@f>t2#d+ztAo)0ID<v<pIp%`3ul}4T>5<KJ`ha"
    "y^0lyM1FuyR(D527d@5^!3x>C+s+wr5dUQJ%-%9Ga+}Ic43WyB&3A~2oy#1d%(_tvL_-;5<O70N<#tfhWjvvpFM4QVbv*ws`O-sI"
    "f-BVs5nCCi7^c~Y)}m>cqBymP%v$9n_;@GpY8Z{UE<Q!%_)Q6!I<wWlvqa2|e!02IudiOH?i309;Q*iQrfs3g<xL-rEMNV$T~RU0"
    "_N_{P?6rgiN8|kSM>8_XhkumsT%3NIt)HxKp6_U1riSTkY;^5g=;}QJv^e^@Q9qsL{a2ttEK#KU{;0uD_u*;IAoO_b_<iPqDrc7r"
    "GVFvS=SYOemWtaYdvC#_W3z)r(^P9PMt8C`{S3Pt5s9NbW5#`#o%5>Fcm-r2e83aqo04;MS(8gkoEw~KleG*V!F^7mw#O1q<5$Gs"
    "=1K2P&oGg>1r|@X6-qXr)x+<PtNC^(YsPQBuy{0G1lkqug{ke&P?~Tn9L*ZUWF8${W6>E@{vOkYs}E6D6u&4wj&JcZeiPiagEvn4"
    "`;+X%_2KmAcb}H4{;9;VZlCHW&)$3g$+K`6SCs_h<cW57L>NX~#!7@Y*V7)%4Q7}@;~b3RnSH7c+oVL_lsX^0N{lBYUrGi!AG-38"
    "{ea$oa|45~E177P4AB=S38aJiuJ@Zizx!dDvwZQgYoRNj=FdF))kjb4&1UNGEu-zCYz5tmnR}-K5(*$b$=AY-z!SD8jk1Vv*d7rI"
    "Ba8UhMikxn{a}!9zkXrdkDYk-^y#d)?~(iEpECOGFdTaM2T3!X-gqRJ(h_NarV#spglZ3I)K2X-j}!OeC385q%LTK?2S{{U#1Ei6"
    "SY-&194O~{4O4Il%5MLq?#0os|DtS(PZ78)i~Ia@)kYkz_9(H+N)sDhwbKD^Yn+7$&6;K{!sTZM3uT|IwWpt0f`Yq&L9Q@g;of29"
    "d)aOX#%f(s4mHlAAT!vX@{k4p7?|R+K7+VjBgXGeJmYc3z&!-F(6&hOHP&*#&h*J7t~T>6J$jnAT`ksl{{eo8`Sc<s;M2ydXBwgq"
    "qYv9uZnqI`Ml{8(6;MwPBYICuNgRrfZxlXg)*_ZT{^Tbz8QC(~TP|!?nEz-Y8kh>Sb=R_wJPcaevO3sa>xv`Z%H9KCA;}AwfV@*q"
    "2bzmYk0VOLSWGQGEOyY5VjT{BXTHwEElw#vT{W*aHL_(wZa7-X^_r{)g|4FfrHV8X=^kK-K72eSt!MBjRG4AS**RnzKZ9(6@CJhl"
    "QBxq%m9};AdF}-s9k7G=I#!-`nM>A?--BqWZAaFLJ-AlV*PxdGA`Ui9^W1C7t*vYej-sjNIF-V@9~djJ3M7PB<9kW6)$1b9&CPUU"
    "L$`+CrBOqMMU@Q@(v`gC;noA%*9coPdMS)yKZfCqO?7jmhmD}|jU)W+e$2ieJo_5@l<N^mH@KZFzilezjff^FE=TpKxZ{NhE%S*x"
    "&3x9v&wOOr-7E|Juf$&Y;PQiHx~b`aEST9nnN?*{b1CtS!;%3M4pzfJu<S4tB(ob9#cwd773c{*Ot=douluBM55g7oD$}hF*$IgT"
    "^G&IU)-=Z=l4pkfqh&_x3xP{jiwrZ*2g$TEo!qR|ZAT6Q^Mi(q{j~LRLMGGHCuU;3u*AAWbS#iB7N_loL_u4z|NcS#@AcIOy0;R6"
    "X@+g_&U`+9M|T&25bCu`<IGK4_a~(#WE6)iqn8{)Dn5&bjKiTYcsbus-c%jESP{A=1!PAhq+3#nkRXVncvGz%BA{C*UUX8j_;mU6"
    "cb_a}@B6;mBp-giSqfp?9AWcYn2BLa3@FBkXo0Wjwc*71+A8?nd>WAs5<pz9Ye+~6b2)IBaZIC%cBg|a;=iPu<!5U<rgWnER5AhJ"
    "N-QiKsD4)ob0g1O)`P7KhOE<X@;i-IgwSCVBF2R!F=vX%Oy&VkW7ck?O88QIS2ehUdPSkY5tZLm&Am8nsx1`cfq&NE3cOtXFEzC-"
    "S=pb2@b+fINvuLWAYrd#cYrWo{`~h}eeub1CV$)t*(@oVmOuaatKWU{&kKGfY^YH)&775tHqHry5}_aOXNW$zA}0PUhc2S8L|V=b"
    "ie&}~T&ma&`&Y$g=>9cvHkcJYSGl8mx0H<}Gguaztc3iK2!;+ccb$6=!DE)3yc25zs)7NkuHx@PaMCRuP4YUo?_NsWjgemUTiecF"
    "Ym>WWV$YPoh{L(LqqU1%sT#yCBcDTeqPA24TxfT@m6tblH=Otp)Z4YNXWhFChB&mF0%FIu<2;!=S)fPeySZ-<HFBbgaRJH7gUq#8"
    "d&xQ8G%!An4OKpTHq~fY31!<_1iJeA<}h18l3WRy3MbuyJcv;jma|0+tTp*qYQBycd3*;J-&rm=ytd@LWB-9yqAtC!(z%-Es2OS%"
    ";X9uL*`(4EpNeI*zJqF*&zHUxu{AUxceGb5Zler|Wr$2L57>ffSxmd!uj-p4Q748P&~)}aiMfNm&T@2!%Y;28bLX^d{?0njB_iLn"
    "dsQX?u`rOf)=d4TJRlh-UMM1vj1d@s$5-;Vzy9|W<^z1oLfN{gp*6GjIql||8*74@A@*@*hA&JHO;0?7;R5hNBpR|PsDg6vhfHt|"
    "S<h^X2Gl$*I?56_gNZ5;`$jSiG6QOV<6PNC4}Z>zmtQ+I&k~!<jK%0@miRb~qX#M3hP->^SxOrP7%f85Xb9jebOa#c>QYEu_-L(U"
    "qGoN0KmjBBo;Jg$!V8i)YX(*3`oroO5|b4ahgv1>plre%u9G`M!LZjXKZM4c^97{2<7<mwMAFUqHb~Zdnm!GkRLI9%JT2d2y^}E9"
    "`1Lj@VjE^SYxEedT}{Uf)KHta6Gf}+e@kh$b<4r?Xc~pINTSHVa$C(}$JAD%sVw+5*%Z~hUkEA@$QIME&%(5mNQ<Oa<b39PeZ<5Z"
    "^{`3re($u%$dBgeTFKqZc7NOGk~WFR)MtXwB_hW_p>?kzXDpf$*P_5CcnAZcQWcxM;*5fTQ-rC^JzaQ2m1euvzIm5e^t$+M4V$5T"
    "5LpGR6$QQh#zV&pSW$=7-TJtP+bpJ!aB}%1a<Y#4C^vI>=QBkd%B$Qt)TOKEK?lMU{&2}?Bp_2SCeq9#PkXRnFe#GRTSl`bka2*b"
    "Urk>wiD;<X-Gm7Bz@Z;(J&EEV|KZQ6kj!;&LSb1DK4N*E;!+IU)*^K=3C?blzb>{sgcr6(Z3}W~hW?xqifx^y29i&pZmvC<MH>4j"
    "q?hM~iUPuF8cPfJaC_va|Mu7aq1m_D-~ReP@vl|6I*EnKf68*@6Wa*ML>};ABx^ElU=)*Mb2L>OYg=`#0E-NzQ!x`9LU-e?swoF0"
    "_6^j*ZHY)e_*Tw(&fN@=2yZ#_)iR6at}=HPLuUyyO`yVMI{9;vy$+h6o)N8R8ZT~5iwi|cHt8Y<yFq%ApxH4+aFALdRGyvJMVONl"
    "W(XY5n5j@AqNYj@q3A$-ma|^d7UL!cKO1qZs#6Plh~Q6#J82mwc@%(Bi!IAfB{r-PyKrj9Y5Htt`!a+0Sw2{G!-l!7UL!7oQ5+I{"
    "Cu}D5eqj(bgR#Y>tQH*#OGR?{d`|_|7g#W4GzFhrh1OSws)=A8GK_hi*pMv!am8FxO)GNaAf$7Mm_hY?lx$@R5Yx9qzR9{h(59q0"
    "03j|KNp<(dG-)7Sjt2s;Jws~x3)>ycMofn?%y60|c3-zNzDmVusN5G^g3K1dEVE8zAR~)0VOlmRCr}WYQm7{&J)6&2i|b#nEy|}v"
    "nrRuMKm$t(Ydo1zYN%2+($(y$8Fl!=RNKrDm1<LnTcL+Pk+AtWk}8QZgLvzEwJ;D$`dwy_n`prozxANOv3K$@V_o*``O6nBO+7Cu"
    "shEm7N|>AGV6yvY?vRJ*#*ZS7GIRXmT3<<wrYq8stDrM83;2t~k0UMSCe=iPq~tkmmai+xl&GI%p<_)L*g8WGyX>S1nP<1=xj&hl"
    "@c)YcdAdaOG644yX=*jH>^LFHb~`dbtC*bhyb6Sr+D<f+8IeBomr`_r4z@;e5Wq*))km%qP~Gr_%l#c5E9M)E6nt4d%wNbyOGxFH"
    "VlH=dc>f;_UYvysOUttCP6kPYFU+F{obSFCDMCv+Tgc6>U8%$3PCuoe<b+Kp3RVtrD%jZh!59H4RN4jK3^{rxE;dZ4sl{y3bfbGN"
    "AiPwBpUu?Ri(L#<5Tg>%43gl!ohdCI=z|LXaljYb5dn23kr;O+LzrBl2_ORNf?p2AS+KXyVA<`9dABX}oEXFr@hVHgo~0?91fY16"
    "&v;Nc&_v=Y!1`MM&T3J@FI1iYF2LlNumL3)<b|4LL<&&GP&Nd5$^@zyP(txrXi<c0GH8%#c5zzTdvh5aGg+(TT2+1;QPA=&8AgjK"
    ")hguQYAKsVmL~<0ALDflWwHF`pI&_a`86V>nrl+%g$|k-4_W}C!BYTdYMR$3zQm%GxWYI54eZI@Hi$Vm{Z_&bF;?#nMpE$iNS6~v"
    "ku3+lZ_gE<o>_>YSjKyn@s_%G`(uX9Qc(VIX%UWqF})@ER%)2y6Qmd9lP>dfOLDA3iw~-d8GdiGq(2I^(iZC8#RG`5tu%?~X>W3h"
    "${aA;EYdK-gABCjr&RigA7syGz-k8CVMUWDgeX;X)&#t7Xy<IynLs9554|%>{QPxl&dA<xW@C$SZW}1$l>)TWpob0D(;H<DP4&5s"
    "m^0=tmkBYOuS0ZAkEr6oO-}waY9H26VUcgf3@AWeNm=y`rGSj|Yy{U;=r*=_Ok%~m8pZuNDdtOgiULBTB`}4b6kmOvjB(e?EaSR3"
    "ps45(Ih?!<LydZ1v-sYK?7=b;D3+(m4Yl6Wp7S6Cwr-b#(Ph`2f^s$4icum2)m3vFrcxFAg&K2D&Bc1A0152LCdmUv#9}dLgzMo_"
    "qbHHY0JgRvuFN!65NV_pm=i-W65?{4OUB2gzWf=m1n)g5Tr{o~**Jb^X0V6as|LBLr0nTD6T);gvcuH%cApb4aa0mHC#poez-U#X"
    "LLNIEi?+4LJmm=bUjEEQk5ce*rdVmwF-=qs{TrwIsm0+YvVlWu0B>h-FhoO>Q7}iAePfs&cPz7E%p;Yj%sTQP#$as+QXjPND<sTJ"
    "lUGY6!)r*?rDa6Ee|;4`%j>IW@=saMM*ezzm5=rM>W3d>ha}YHf!)|%(W-aljd`5N)aj<-^wDI1N{EufUyqGl`ixZ-KPDP=7k*3*"
    "zEJbEWLVCcGiZs`p+xJ30g^DtT}w<q`RY@`V%~qEVl+GCRAw3TW0r|K>HrQu9Rg@n#-#w`by)j~e7LV%FyA1CsT-J#aRWShU59Uj"
    "hsSe$^eLmB*%|Ae&UE9GZxL(Q#GKuR1fC{XUZ?K_U`NRFJUg)uJlv*WZv9RWNy}_t;<zVtukfQ^Jz;J>?Xp$fX<STd*d=DUA=oqY"
    "O&kNf8>a)Ya};rz+d;X3OxtT(&X(dhuO1Ujge1g|vf+!IlRVrT%55+!KMU5wUc5of0OJxh;#Crknqfxg(5Y2vGJUchR3NN4$Lr!7"
    "`>WUb1g5$z?W}@_SuxK!7?Xy!D7_+(=kV(%q@i;vGZpo2%NXfWwW4gj9FK^LC*jozdo>nSKZFQyJJf!aS53$&PqZp_a1{z*+BBZf"
    "zlECV;^xE`;jG+(6##k=W!*4yURqihgva&%-4o$OqHKMRz!E6p%j?}!9q^+ZJzi0AfT8Z+O@)@&IUWD$a{LC}MNO|{@gS@EB{o3@"
    "pmwEm6;M1k9gqT^A-L1#b+L#$SA%;)_gd3jjmnK2^aTM1^BqOgLKbeVT<a^Ji@k6JfonOdy%o>WmQ?o#PRuD%vIIFn=P3TzRX<D("
    "Jf=%!h_bjn-AG+~Wy^bM0bN2Y#3k4%^nzh=^*fb>6{KsKAQxl3;+PG@E{fQ=j4qN9e$KtBYmGH`@c2@4Ixna<(p6iXN9a34hkjEr"
    "2W8LC*SD<Wf971F9&@H4dXw+F6M>qLL$o@Q05tp0%shJ;brC}YV0RKtXx&J@WP6fnDi%RlE|DQXp1?N}mp8%gmYrFYDjkxcsN27x"
    "+01-UA{?<u(QYFnKAMVIXn!^`DN`VMYSV|bGe2TfJEtW98xMi99Ce%9;e++naEH%f4us^hwE(q*tW@+HQ3k2p<eh#jV-5B^Xof-J"
    "Y#N*<tSiHKhuBbvow+Zsn@_R``pEZxOYI9qG(#V0_X!f#YG|DcRuX#W!1O|t@}zw=j`V%;mkVN*be=`Q)$83lgq_ZHE2|Zbv$*D`"
    "Fp6iFmvN!E<QKmX?*7FuqGI%T$+?p`W;T-Qp_HPji4{2YdC2rU=YoO0WP+pKPTykDYdsea#{`n)-O73{Ip~<{PJw!{HU;l<kDq*Z"
    "+}vP=Pi9erjM6WrvbVdVnoy~h>Rn{a$cUpV9Y-)c@%+H#fTz)QoVK3rC|a$0(V=x<rP2dazAM6o^)TAj99t3fV_B)h2+&iHDm)VK"
    "!o5|Don*hmrw!0SDW(0?vaT@^$V|<5BJDU#8+X|Q)ZOWy4uI+NK337l0}Xn_YRI2Yy|>uKM!T?G>|cZPRTGVcHci8l3@D7bT7b%I"
    "*C-(DtC0<@-%xG{wKRa_Dr_(d=J8li*ssy)$d{@UWYJM-VVqAzc(>O7A<o@F11>EayUdC)5sHUBt_zcBHD0W3v6d=ompIPF2ve}U"
    "wn7&{da`ykRSIfdU;>=v(>G~@@HhlCmxw@P*9G=PBBJ}O%AAxG=)<<V5fkYzWs0cfqfeIN)@TA@YD|&l$%;s_%@`?RvC&UFd-!|w"
    "0FvW_bnyBjEtcP4dk`W{^^mcn>5ODEpMUnrm#@B|5;jn>`upC##BrvkoEF?__7GgE_X)xhs#Xk)w%_p96TrCwl`iS}$IeIqvWS6d"
    "C|Q~eNWY1!SBJW{3NxLB^#>+rKfF9(MF^SK+}Tf1hZ^;S^wenb!t1pOdmffE`IehELZ)yHZ%~s@zbe<6b(4+beMcTfdOpv;_9n}e"
    "(U8P;EmRKv^GQ`)y`4asF8zKwr_v%OjJg0jTxrVf+q1dSe>%dHo<dHnON2rq<b9QvRhxN~g_S~2MGY&Vds94Cwhuitor1vv!=^@!"
    "CzX79L>8g4eeI}wvk2u#zWDfM2(Uo=SZrI24fz}bdI6}PoY4Yksa^QZpn%A~%iPt-w`UPg3Z<jNFwNvHilVnQ5l$F#us7P2We;lf"
    "1snc>AEZ{oAY8Z7fPT1ZtXE4D(BhKBur?JOsaOq4tS=l~Iz3j-L{PgT|23o_E_}6xpmPDbIx#@ft2NA+vos;1Otx(y`G`xsuHsNF"
    "PR(e?hu&*)kpfKD*Vo0<kJL9^et7;l>b@){(n<?B?kiGDQKXdGA9w~on2G`PMsP;g9uRiChx4~rd0U4X%bF_z*BPv*1_GiqQ*5Hj"
    "+HGcU2AECJ?ZyR|wffYi<1G4&o`Z7^@||Wb6bb|KUA0n}Qpcj1Q6(T0U-}A#Kq9+6R!$f+GGam{&qw0vn>A5=nBo$1SU$VRbO|yR"
    "09$L?kt=t^%}#I%pDK=xl{%rQtxnW2Ae+3U(wfPU2ed6x2w~V3n!#0cTP$!QZQQNFtmoNO131j(X_v};fdM#9xU3wD&=dxn(YhvI"
    "#c|NMT`Ybwo6KEYihQ*fx)%Z+yMyD$dU93YBgDr$y|(Ie>?%%RqEk={Q<cO=1^!vzcWeMGZ}gpArPP&VRZ<2QR~%8H$Q&l;9ziq-"
    "PgVEsIM`fM9MFEH#V?xl3U#^>z!US@XVNB5f1jy5CA5AtM`lWtlmhaW3TPA@m0+?e%UrGafoHLuoPN(KRMeB~;swj2_y;Kn0>iw%"
    "T52Oo6VW!(ZJBzkFukEw$4DbmU}>p&lRT<)TATK`8lrPxHhEUa<)p&BBgThS07{nH_GcbJQTJ5TJa{hQkej1{QHh*rw+@F288>c9"
    "lhkaPbS!rGl&t-E@sCHe`Wo+%;qSts>_v^Do1X=pbnE+*FwUDZGI%I0o~=}emorEum?QM_^GMt*wf8jes>+CZJn`nll!x%a5v*kp"
    "LMDpQm@Hnxk7Dq%NvRFhdSz6;A<St6#YG`73mbU)4u!VX7>qoD|Is}ssV;@M`^*)Bhj(Rut9!oI-8H12&R~Sxlq$Stt;OST8ocX{"
    "&&$f-IIV;b^!_&u)hdI-Njv3^m-cbu;-Clt`nII&zu_R$-ldNW*kL~mJZJ2=YaNK>5%2Zmd5)?~CIOusldIwn`;XE!T4kyK4^Ws$"
    "Fe~9_FkH)YUYHr*2NU~v<`+!2DeDa*lS4RH2>M8b?P*Hm8R~G%Z{E;qducLe1N;`Y%hx!{SLGWPx@3w<(?KZCkUYDz?>iZsdqz0t"
    "d}!zD#RkYwQ(r<UYuJcdx~ai_uy`iz8|k;m?m2G&gXEVftdZPWsI{tz+F*Gq!*;rjMF8Rssee`;clQ8jO60pc9xiD6(M+?^Kr|~<"
    "K7p{<FH7*N^|7}SoN(4hp@!T}+Tawr_Gm@Yy-?Go9pc#$E%)|l0APZ#20j$!{I*7W{Tdw@MKsG4JXer*|7nkO3W!L!eCQ%%l};ja"
    "F*x&m`5lr}qZw7<5t@q8^juG*K2V6<0)qWwHdce-2kpE$&LnCHXrKBSibB3;<e&Pg@e)Kc4&btLJ@Q9dBWsj~l4EJD1>Zce{*cIc"
    "D*qUxJvG}JmSMSz8VEWTIUZ0_&Uv$g5YEiJ%~2%BLt{%7OXFP^pXXm@Z;8fSU3I3X@|PyZSM7n^%~V2}T_AbxfU?&3umd%npSoo-"
    "*oz+t#g1k9R`!={EH(?S$<o%iUq)%_G7vNUv6p_EB*5hCNHoSU-qdhL@Dy`R^#2d)a6?*W!E#-rI5=xrQL1IYQ-#0hwTD*zTFw%R"
    "w)E}*c}Evmp6Fy`53pFD^pZnFT$VWrb5Aoi%DPNkmUt$zNWltoWDJi{eI<9w+1y?{Y5zLJo1zEG7<imhuX5mJ_M4Dvsr_H-emt~1"
    "PK`c(NB>h_!c_W%(R`(fdk(FWGRM=Y)wD?ioVv^)CmCj&?W0DxYFUdp_A~T89+dIVRF#uP5P}}JgHHI%9N0=z9KO!zjIeIgA*MOM"
    "aV7PVQIOZo_uosFrx5{d_m<$MiEVvT{P2VR;Dsd7h3jsVdOnLRm<H3x=P6l{s#ZLMGmTo0aNv3K3JpKi7oeP3EFJYS)@$zA<e`Fg"
    "r>D>8FM&<`EGxRyvWM+X^V1ADKNZqHMg2!AkBQbR+AoD|tt{LzXs1(~>KJiHuSRo`)^Jkux1V@F4e#Mh%E+T;*6WC=T@CD7cEu5&"
    "u1Nx8LU+m_7g&XtDL4^ZE%|IZm=-qg+riSq2@f5iBzpziP!B)T{mh?5#_%Ws?X`lg1dDBchGELHYF^;*JfYTWDmAO^O5w~bn3$`Q"
    "$AxE>&vPD}dBAVPh`#(hX+~BgTWfu_8M4V*@!d%5>cM<VjkCVB2CE`B&h`YN(G=K~SL~|NSW9nc^e#B1PU5CiD^F&GhxAFp@M&e7"
    "whvZ6-yh1$1_HUopm=JwO3x<MkfL{}bTlxVsmW^|qi+0TO@lpWbp7k>2bRGd=gE*jHRz9~!c{dIyxi1nV2u=0Whffnmcme(<%ih%"
    "z`kUmbD!jVXpplRz`4rU9V1Cl9NDzdzdg8Zb_)v=S73c!MiYql-f{};wV?*Iq!TZ{8EFG<aAqxHr8~kik~DLWemtbjfW7I+s6yPj"
    "&e&^<uM`J)_QUaF3@oCHVA-qwKIGA4{*H3fkbxj8mUdE$RFKvGgC)in-bZ1x)b`AMl>TT_vvVs-4^KnJq$%bfr?lj`*o=MxXmydw"
    "6ItWT$=E0s$@HzaKq&s0x-(~%JSgjW+7*pCy$kDK#^o#OJJ0g&I!Y{E^_}OxQOU(E&*P5V1J5ADJfej`mwuuBY`uW(*jJkzux0Y$"
    "E9;%_dRr`=X|9;tU}kUfa<Vk>+p@&-tfgV*btgT^1j!?MU87)dVp23*pVw&k@j^<`a2Z+caUM>Xz98ZTmc%Iwr6;Qv@D>e1sW-HT"
    "oV7J?OV3BR=T-ovy&=xBjbR*A7K$+F{3C(uVb#makEuEJQ=as|YI-h<m0mgp%8LQ|MErQS+b+bjC&pr!{851t=k=%=mQ1!K9)4s6"
    "ZZi8S>`_tk5-6CPA2rH(J{x6gBQ=p_0houGkAmVFLd^or7&)ahm7=qYY$g$M2-qJ9ZhHsDvPtdaOLqhh7WDJToRdz97mmjbN)nTQ"
    "aEsu4Si2V@57Sz|ip&?eKFb>L0YOupd7j=qucSpH<&eVSYJfmQ5vMZZ^@NT>5i+~XW#*%nA}R*zXlTfPmvFv;{wy`Y9vLY&p;<(V"
    "8Ff%7ket%30hbymo#3Urjrtec3j)MEUEIuo3!S{U^LHTqdTTdwXx`~emf#sfmMZpon}tlZdmh9m-z;vIDK=R$Ov}2l0(Ud-zdb`y"
    "kj~x;DG1`oGFVJO{j|MF27tmI_S&$lZlVVLLV8PQ`&w>tdu&0kud*v^q%xYW*?pB~-pb+GR2a3^sZ-NyvEB<)_+To_BCX))4Q0Yd"
    "a4DZ=T``mOg!|B3&d9dSRVFOMyk2J57XZE-4aZ&v1UI|&%n|@q_fa6=H`btvnY|BEpQ%aS++c8V`RA2bKA9yCfHS-pI;j~t>RE^z"
    "rq=0ErEjHsxZgmQa%Im+SPI)zf;MoH=Tyd!kZXB0?pgTEzzTW8gD&eBp=ObSUe((EvK>uko?%lq=!VZ*nsCzrBEQl7g*#Vwn>i9B"
    "CkfG&-;BexIZ`S!!Xs#Ct#a!djbXcem$)lx_&Nmx?U^Xj5zPywM>-29ZM>;=>4b&N!mX{V(cIkrKKnkAXPW(NBCqHZuQh1<Ib7KJ"
    "eLshyAt3vixwUQsF>9UW_TE8N2g)cci6aUS5eW3CRq}=@oEleYgQZO>P4QNCWj8J`XK0w6*|fe!qAEz0c%STXMup5udGwS*^=ogO"
    "jUu_<A#D9g7FQ%;=E(wHwJ>R$_;?7P_jbg?mG*XFoj5Yr8pJa$1N`Rl2+u~x{4_h`h|3$bN*t;_XdG+C`v-%10~QiLL7ETRbACJ!"
    "72<&kc>|;^?5&8tTfv)>-(~81`UvSM2v8ReJFq#0#;3I3(_Hdx#1$oKTI>JTUif}!hsyJxD|e`)pLs;mRjO#f6hp3EL~3n%)G3r&"
    "BI|uMSqBk$ix0GJDSy~u${OrZB$4f(;S!W8sp&?58BeMh)q3mvG-FB=Hf7-^85DftRlzH#KMKHhY2bAZ9Ui&CBhcJa-ZQsOn^tLU"
    "bs2eod(H9$h)2gcchg1%XW0~AML&$;$or#a#{8Q~dL!S&T#j8d=t;uUuRZAO0+O&uM^@U&Rx@F@EbRn415;Bhn$^TxU5oX2^5&T`"
    "k*!|SRAvy)2$X+d&CyVcuL+$vA&KVV?t|#x4~$-l5BN=}nzxy>Q&oD;&gmh)!9zohATX^P=RG#OyXNi4W7P!;=IY~!MPRMRN6eq;"
    "h293ndoHsv6j&l0@EpiWG1^wV)1R*d74-43F85Y@m*m{Qtp~ZOxRmzH4N;zJ?P^81p4LG*bjW`BP6`LBIdIlP?o5z!>8ic0xt*;|"
    "-s54f5^yXn$xtUKWb41`Ewq-@AuuVeo?wQP8V@zJS#wrzYZ+~#&LBO8g+U-HW3uP5B0pvACt64H!iqk}#}(d}H}x%JoLckILTjh="
    "BUQWxPh%sBi@54cU!}`ygFYN~s9TRnWSc|T9D;34VN-V#YtPAH926WG<oxt5?7M;~(b<^3QNGGVsg9*p>){fpn|g@NpT+&G_kG5N"
    "K8zuBtX;}H!d7rHpPA>CRNHxA%-*5`t)H@2!d8xuFqs?0A4bw5ELPW>qu|z*#{W0xnzq$${IV8$yrDDAFd9eXFzHOQXb(B3J<|YF"
    "!hK}A4|*?%_$7oI`N>(uQ4%htw+FAnSh0d|ZNwJ!mr-k{-%@C~{0mWb)^{{PYtv7)8slvn&KTLm$E6h+hj+p5o26<z2~!x{35=)V"
    "M@BjxeU}NY3smJaGR6+tiu{<}Nvy_8`-xLhL;B;CfEN#OWhiC^nCIwq8tm&K2aqUmh^joH6DfM751wD3=#{5K8|C3CD5a4#>$jQN"
    "2o&)Wr#KU5Rzh($y5=IpL$n)kwb7^VY(8g=(`MK?$AN?ex`l&1DFG1^<$?V})rlTqrlf04pP}+6%Sc_2?D=>4e8F=cg%?7U-b)~#"
    "Gl*j5O~n1PgPxTqLA>Z9U#Z{5V10``6q|SG@WjlSH)W)vW&Nf4`>onYnb5G9;<~CmlBxz@2JlkC)4uU%v(GMY)BB~zah^mc4_aMW"
    "ZpKGRbaZo0Z7I$(+7E`S(!HKJA~%cu$`uXM2BYWDC(Navs#>;6pFo}94}=H>`)4zHOo(3CsJDjf6V$?i>yHYY%haVzk<l%a`4fOj"
    "ID>}LgxnTZxbf;E?`U+EIT9iBGJzBW|M3S0Dbh%O8;a3x%w({_i|Xx}-md%@-w%PFHj}%2oXt?Mb)<VBpD+lCXnHv~8bRaZGF((!"
    "ME}e4j$Tt-MathvxN(2r9o5<f5@f;7ZaK)wT|o1a6tab<W*Mq;W^H9Gx-!?U$E1naAB7C5-|7n;*_uRXmWrD=3uc&qNDN)wqfC%{"
    "JWcWdL7>PH0S;;-YpYF>OX^Ft^aP4+0(RV>&=I0pNrc0M9352r6Pg)_MQiih+Op@Ksz7Rwxi`W}L?Ubiu)leVK85&_RS~5AIdQ0S"
    "#i81gst|V<_)Sj+?|Um1nw4=Wl{hVgaI*AX<c9aKvx(E!vbG#H8yu6jB7L5+)}s!k%yT>jn01<rMuEJ8X=5F8Gfy`mN^q|gvt!l)"
    "5NfZ|R#I7b&tbH>3f(&txVW6)Z2*9z91qreq8d^cDs85p<2a}EIFhs8irZsh|51?Be+R^<0|dkPw2<Be6zIiuVd;4Rg7eR`vQd$|"
    "7O35h+n_dt6uj6Hc2QY(&;Sasxic_;4w57$Ve-Dra=EF8BD?zb48fPhdo0tx)fOOGq%ntsRwV_k-c(gs`>}@A{0d1y_;H>Atn%zV"
    "uA>qxSFx14mrwvXhetj~YFaWXhhu;1HPPi@Z!i(Uc)SsnFP!jJs(Fb{g6W=m&|iP58>h4^FNNI7t}@F|sCaRBEbbQ21c@C`bX%(K"
    "G(j@=9F_DdvFrPIkS#0@u4m)zGy;PV*R2q2QXzIMji<;_{ZnhntCz|qylCBXYqgtCcd{lhX8U<>lEE%zgG7BZGhWk*+%kmq<SrXE"
    "LCXxVue<ZqE}Qn-XS7G3Y`^&&ZsO$}mG`QD3!48T>%hbg!;q1g2hR$R@I_iNQ)}p<l{jnut)}8xt-42X-CZ>X^m!)J0+w~dJKC~K"
    "PUhAsZW1Jycn?~v^^=y(_QMRvpP>NRi6L(^;?Zu@&1%Tn`mI~fUcNRuT$l*be%-yrooqvR$v8u#257pD$d2Z;vdaL%uuC{$`W&>("
    "^AhN|UCTp@8@<+nm$b9tahjXnW}|I$Wk7HxNSW0?^Q?|O8eFnnpoKzt{iF4`nilCXK6gPCPXuIAJgC~8r{1)zK|8y}g_pux*JQn-"
    "Fl$k--~(?0^W5;=XSVlbLa76qA8}R}LgYZT>Cf8kpZWcxhH-DTV=k<I0;l+i+~+`i*)#~0?PK*<7iMyK4jF@}y}BXla6&XW&4fAh"
    "xg^$^9L#N@+)xPYrxrmx&Fe7w%QY7NM=An-tOnykT|<g<{A()&w0ovuk>3i&*K@CbYQX@Wx?!}$@&9I+WzE~QzWm2a3k;{wND5!t"
    "<s5d446(plA&-NBcz$a^7ug4Po;KJ43`h-6m<TJ@`L8@wqo$t*UzYJUbXi{?OaI%r%HNb4JiNP%zZY;P%X(o014M`B7NR*9V(azG"
    "C)bX?Z1kWqp&I?`BjP3|JdGfw;*Wvaj_VptmCQ$x4?^o2Zw(d6u2f6+T%)R1&p6jij#>QLBSbBDbJqg_tqzX^PGz7kvldkO#nW1J"
    "Ft`8G3W@p(-|aTKb7jAr<&_F)Y-%zF=;WoZ9)028{`%i8e5xZF;FfWf-bb^VjqI-Pad1rQm)VfUl6(+|xS7O}cZMoIgE2*$ES0`N"
    "rDo&J+2qPTquHpNGBOR=;8A@d^j`*;gSJ~Uehf<B9ErHm3+`pHk;lBG{o2kBckbTA_=E#wA*5%*ZccSDukHGg^#y{&FXQPlr#60*"
    "MCCkdtnEg!rvP$kIYpgi!>`9%n}4;_yh`&nDjg4XVJQ|7b{MySIy2;*P>NoRJ|>fOcB@W<E~@o?b%_z<&2o4<i=t&HK+9tX4BY}d"
    "FaZ1_O*Q7D;j7RA|L)bS4&gl)nR@RPQP!@GZ?ZcSaiVhLGqW}tj>@ZWtPBMKnrPY>VSLNy;ce6z^DJoawq5s2{}M4OASWjlUs#6j"
    "J`bZf%O%1Ev}s)*YPE>7mYZQXTU*aDQzj(gnSnS)n8RNeu+zw@(~~TxiDJ`w#xVvD8^`<A6MqC>Im<w?5sLPPLu9{;*%kSFMHohQ"
    "sX`i<(b038^lb7I);i0YJ)Ux^E^pF>*F0EbC=+SQ2-@cUr|?1l6S&~5@85<5bbXj|Yxa3&{Q}gN8{iijyM)kG@EKpu(RaKKI31^<"
    "bQr1H@=lM47i)(NYago}(i*>Gkno$VEZ&R6{jR)nBF3b_gol{D`Zq2*>|JbW|6Xsntg*;(pYIFJM>P<nCSP7JHiFUQ=?)82Ro%(6"
    "#nGcEk`k#mVQQ**F8%S|JN50<;29MRV+^mE$oe6|(e+Gd9yoI4nQz9K#Ij4%cM9vO;|6BUWvPyCB46z__Xc79n0B&8A|8ta8#zqi"
    "0}h)=0!_m`o<I00Q#T&tMuf1;62kf3S&jPrsEb!R38&D@Reg5k8Or5r%O071GtX}qOhx&1bIW?aNlm`q9o3F%Zfi95=)E|5&8SDh"
    "hea6qG2*L-xazkbBWfcr+-&l06)2upY-KVM#U1%U{1NuHUeEv>^$d0EDen_134z55oq7g#`mEU5WFvn|H)G^#uVgL@3yUp)x9t~f"
    "UWkFRrzBE{09?0-f{q2p#=Cy=-mib1KVsb@N9{yEH0bm~B!#%r3xD_{Yq^<j{nJuXeT=)Q0A{-<5uiF6)ZgQBguQnC|Dn*iSWfKP"
    "C6k<aX9+^y(>xj*jkfO-#62!a<Q^oT`;z}&rmODxDf)HI5;%MoKWBw&he@FyieJ9kr+R5y*o)s8gNb`Be<SWi=@eE@jwH0)wguRD"
    "71k_AF{RX@S&mOLw+?(}X~Ztw0{Er2x<>OIz898iYTkuurLuaxkubW0&U7!l-RF#%0VFW77U`e7{8KHLu1F8RKQHwS@N4$42TNZ3"
    "p+E|%0Q@5QSZehy?N=>u)y8v`HRkTK#A&Pjv*VgeO9SZ>=SR)jBS#ySa(DtQ7yF(8aq=yE9ai|ZyLg#}<0Y2Dng*1MRR+B4;zHu$"
    "Q{CC|`>gKqys?%ix^jUwic;FOFujUFPRVqDUO?jAo%(B8<-*>&REx>3=g@whVAAO>yly^x`WPD`%z>`mXnp4l48BF=c(6vIrS<<j"
    "XbE?|V7qZ3c!l*L1QMi{$_&}?)(Q_E{}e9L$X+ngbts_6nTeX^E!^adx9tQF?Gc`;wTISE;VHXB)2=L9n)EaoA&Zx8e|I1$FJKC>"
    "k@cI0VC?>{<ioH1$`!W5*LzpSSKCL30dNEn$WcqF4}(Y)kZEmBi9mTr8B1ZIdh@tuIwdD>^-$+yYddg@>y7w!&>L{YnzMo~liR!o"
    "n>VuoeArvo?UXJ%hLY6qN}MoMY&ug)2_CE{P3N(sY|=4HY|K&Z_MPgXLiH`_a3vP4-sNoVg1+*ujH5?c`ezDmmM)gtv58b55%c{q"
    "(jQ$!^(gD}v2i)LNz5$jNY-T?-H4!t+P@+E_EOJ5RFy{=Lwa3!Mu@MQt(2NkOUCe}TaQzHzsYW|!+Mqz!{MVPp~Q-^DdXpwxo2aY"
    "F8aD@#l0#7R|G0wOZ~x2$y9HA2;t=SAlT)<7H~f2a^FHM(%W7b$6x#Pe3u0j$lhSl*5YM}7cFYu>ggpSA7nXf{KDTs^oaR9CU=oZ"
    "<*$3Ykh2%@xxQUE_I;^RX&v`bU?Sek1945oFL<L9&ytpXLNlyHrrk--nnY}=2ygh$G<MynSt|~u;-IvJxP{Q!aUNd1NYjO1q_NPV"
    "${(>0UyYUJ#5@ePNj><T9r9bHZ<yY(w*5jo4}L`hI;$FOykhxpPQ^c>#I^V?)84${*2?>dHJBPQ)_x<{t<n48dhLyIK~w&Ji}OEF"
    ")2T3E)i-J1E<2a394;7qyUi6BCut_w`YMIWli7_#>g7Rx)FiviHmf7r)FcP&z2E%m?|=OREja73egA%b6GUU>$#M|{6IgZJ1%V#{"
    "+$9$tP>U{5i7h8v!M)QfCjRti%)=WuNgMwKlo`LdS|O$+G>ECnjVSgXdEnT`-(72eYG8o>XomNHV}GD!t<e1VUyJqXuyC%6-{fBo"
    "0B~RUoVVUA*hvZB3cthU=AVKi@&7I&^+~&zGrV2nfA8LVpOwkB0-L@3N7|`Y_UFEQgTMae*h)1y{_58?|09{&1wH(QZ_Fb5<)yDD"
    "(-ri<{tEkT$IvfKkmE39$0a~f<=>8u&gdv}+6_3gFfiCcf=6AVi~Lg_=l54N0t2$dnzz|8^(nXp`T>IbyW)^>djdXw@A~iFAQ)@C"
    "z1b&yqgM}I{XoLmY-+{8E%;b^+tn6BjusoY&@c4Dx6=Z37ke}|zp(h}(@NEN>WBXY^v=7F"
)
TIMEOUT = 45.0
MARKERS = {
    "pre_compaction": "Código anterior à compactação: JABUTICABA-781. ação\u2028🙂",
    "rejected_fork": "FORK-REJEITADO-983",
    "summary": "RESUMO-NÃO-IMPORTAR-294",
    "middle": "MARCADOR-MEIO-543",
    "end": "MARCADOR-FIM-876",
    "instructions": "INSTRUÇÃO-GRAVADA-607: preservar ação e café.",
    "file": "ARQUIVO-HISTÓRICO-419: conteúdo antigo; não reler disco.",
    "error": "ERRO-CONHECIDO-351: arquivo artificial indisponível.",
    "unknown": "RESULTADO-DESCONHECIDO-852",
}


def json_bytes(value: object) -> bytes:
    return json.dumps(value, ensure_ascii=False, allow_nan=False).encode("utf-8")


def digest(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def write_json(path: Path, value: object) -> None:
    path.write_bytes(json_bytes(value) + b"\n")


def require(condition: bool, detail: str) -> None:
    if not condition:
        raise RuntimeError(detail)


def valid_png() -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack("!I", len(data)) + kind + data + struct.pack("!I", zlib.crc32(kind + data))

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack("!2I5B", 1, 1, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(b"\x00\xff\x00\x80")) + chunk(b"IEND", b""))


def create_source(root: Path, *, oversized: bool = False) -> tuple[Path, dict]:
    image = valid_png()
    output = "".join(f"line-{index:05d}|\n" for index in range(12000))
    at = len(output) // 2
    output = output[:at] + MARKERS["middle"] + output[at + len(MARKERS["middle"]):]
    output = output[:-len(MARKERS["end"])] + MARKERS["end"]
    require(len(output) == 144000, "A fonte longa deve ter exatamente 144000 caracteres")
    if oversized:
        output += "X" * 400000
    arguments = {"file_path": str(root / "recorded-only.txt"), "offset": 17, "limit": 80,
                 "literal": "ação\u2028🙂", "nested": {"sequence": [1, "café", False]}}
    image_block = {"type": "image", "source": {"type": "base64", "media_type": "image/png",
                                               "data": base64.b64encode(image).decode("ascii")}}
    rows = []

    def row(kind: str, name: str, parent: str | None, **fields) -> None:
        rows.append({"type": kind, "uuid": name, "parentUuid": parent,
                     "timestamp": "2026-10-03T00:00:00.000Z", **fields})

    def message(kind: str, name: str, parent: str | None, content: list) -> None:
        row(kind, name, parent, message={"role": kind, "content": content})

    message("user", "old-user", None, [{"type": "text", "text": MARKERS["pre_compaction"]}])
    message("assistant", "fork", "old-user", [{"type": "text", "text": MARKERS["rejected_fork"]}])
    message("assistant", "old-assistant", "old-user", [{"type": "text", "text": "Resposta antiga integral."}])
    row("system", "boundary", None, subtype="compact_boundary", logicalParentUuid="old-assistant")
    row("user", "summary", "boundary", isCompactSummary=True,
        message={"role": "user", "content": [{"type": "text", "text": MARKERS["summary"]}]})
    row("attachment", "instructions", "summary", attachment={"type": "instructions", "files": [
        {"path": str(root / "original-CLAUDE.md"), "content": MARKERS["instructions"]}]})
    row("attachment", "file", "instructions", attachment={"type": "file", "filename": "recorded-only.txt",
        "content": {"type": "text", "file": {"content": MARKERS["file"]}}})
    message("assistant", "read", "file", [{"type": "tool_use", "id": "read-call", "name": "Read", "input": arguments}])
    message("user", "read-result", "read", [{"type": "tool_result", "tool_use_id": "read-call", "content": output}])
    message("assistant", "error-call", "read-result", [{"type": "tool_use", "id": "error-call", "name": "Bash",
                                                        "input": {"command": "artificial-do-not-execute"}}])
    message("user", "error-result", "error-call", [{"type": "tool_result", "tool_use_id": "error-call",
                                                     "is_error": True, "content": MARKERS["error"]}])
    message("assistant", "unknown-call", "error-result", [{"type": "tool_use", "id": "unknown-call", "name": "Edit",
                                                           "input": {"marker": MARKERS["unknown"]}}])
    message("user", "image", "unknown-call", [image_block, {"type": "text", "text": "Imagem PNG artificial de 1×1."}])
    rows.append({"type": "last-prompt", "leafUuid": "image"})
    source = root / ("oversized-claude.jsonl" if oversized else "claude.jsonl")
    source.write_bytes(b"".join(json_bytes(entry) + b"\n" for entry in rows))
    if not oversized:
        (root / "artificial.png").write_bytes(image)
    return source, {"output": output, "arguments": arguments, "image": image,
                    "selected_uuids": sorted(entry["uuid"] for entry in rows
                                              if entry.get("uuid") not in {None, "fork", "summary"})}


def write_catalog(path: Path) -> dict:
    raw = zlib.decompress(base64.b85decode(MODEL_DATA))
    require(digest(raw) == SOURCE_MODEL_SHA256, "O registro primário embutido divergiu do digest")
    model = json.loads(raw)
    require(model["slug"] == MODEL, "Modelo embutido inesperado")
    # ModelInfo aplica exatamente este default serde na mesma revisão da fonte.
    model.setdefault("effective_context_window_percent", 95)
    write_json(path, {"models": [model]})
    return {"commit": SOURCE_COMMIT, "version": VERSION, "url": SOURCE_URL,
            "source_catalog_sha256": SOURCE_CATALOG_SHA256,
            "source_model_sha256": SOURCE_MODEL_SHA256, "effective_catalog_sha256": digest(path.read_bytes()),
            "materialized_defaults": {"effective_context_window_percent": 95},
            "default_source": f"https://github.com/openai/codex/blob/{SOURCE_COMMIT}/codex-rs/protocol/src/openai_models.rs#L389",
            "context_window": model["context_window"], "max_context_window": model["max_context_window"],
            "limitation": "Catálogo embarcado da revisão fixa; não comprova acesso nem capacidade atual de servidor real."}


class CaptureServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, root: Path):
        super().__init__(("127.0.0.1", 0), CaptureHandler)
        self.root = root
        self.requests = []
        self.failures = []
        self.lock = threading.Lock()


class CaptureHandler(BaseHTTPRequestHandler):
    def log_message(self, *args) -> None:
        pass

    def do_POST(self) -> None:
        self.connection.settimeout(TIMEOUT)
        try:
            require(not self.headers.get("Authorization"), "Pedido inesperadamente autenticado")
            size = int(self.headers.get("Content-Length", "0"))
            require(0 < size <= 8 * 1024 * 1024, "Tamanho HTTP inesperado")
            raw = self.rfile.read(size)
            require(len(raw) == size, "EOF no corpo HTTP")
            body = json.loads(raw)
            with self.server.lock:
                index = len(self.server.requests)
                entry = {"path": self.path, "body": body, "raw_bytes": len(raw), "raw_sha256": digest(raw)}
                self.server.requests.append(entry)
                (self.server.root / f"request-{index:02d}.json").write_bytes(raw)
            require(self.path in {"/v1/responses", "/v1/responses/lite"},
                    f"Endpoint inesperado (inclui compactação): {self.path}")
            events = [
                {"type": "response.created", "response": {"id": f"resp_capture_{index}"}},
                {"type": "response.output_item.done", "item": {"type": "message", "role": "assistant",
                 "id": f"msg_capture_{index}", "content": [{"type": "output_text", "text": "Resposta simulada de captura."}]}},
                {"type": "response.completed", "response": {"id": f"resp_capture_{index}",
                 "usage": {"input_tokens": 0, "output_tokens": 0, "total_tokens": 0}}},
            ]
            data = b"".join(f"event: {entry['type']}\ndata: ".encode() + json_bytes(entry) + b"\n\n" for entry in events)
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except Exception as exc:
            with self.server.lock:
                self.server.failures.append({"type": type(exc).__name__, "message": str(exc)})
            self.send_error(400, "Falha na captura privada")


def project(actual: object, expected: object) -> object:
    if isinstance(expected, dict) and isinstance(actual, dict):
        return {key: project(actual.get(key), value) for key, value in expected.items() if key != "id"}
    if isinstance(expected, list) and isinstance(actual, list):
        return [project(a, e) for a, e in zip(actual, expected)] if len(actual) == len(expected) else None
    return actual


def without_ids(item: object) -> object:
    if isinstance(item, dict):
        return {key: without_ids(value) for key, value in item.items() if key != "id"}
    return [without_ids(value) for value in item] if isinstance(item, list) else item


def inspect_capture(request: dict, context, original: dict) -> dict:
    body = request["body"]
    require(body.get("model") == MODEL, "Modelo do pedido divergiu")
    actual = body.get("input")
    require(isinstance(actual, list), "Pedido sem lista de contexto")
    selected, positions, cursor = [], [], 0
    for expected in context.items:
        matches = [index for index in range(len(actual))
                   if project(actual[index], expected) == without_ids(expected)]
        require(len(matches) == 1 and matches[0] >= cursor,
                f"Item importado ausente, duplicado, alterado ou fora de ordem após posição {cursor}")
        position = matches[0]
        selected.append(project(actual[position], expected))
        positions.append(position)
        cursor = position + 1
    raw_items = json_bytes(actual)
    for name in ("rejected_fork", "summary"):
        require(MARKERS[name].encode() not in raw_items, f"Ramo descartado apareceu: {name}")
    call = next(item for item in actual if item.get("type") == "function_call" and item.get("call_id") == "read-call")
    output = next(item["output"] for item in actual if item.get("type") == "function_call_output" and item.get("call_id") == "read-call")
    require(json.loads(call["arguments"]) == original["arguments"], "Argumentos históricos alterados")
    require(output == original["output"], "Resultado longo alterado ou cortado")
    require(not any(item.get("type") in {"function_call", "function_call_output"}
                    and item.get("call_id") == "unknown-call" for item in actual),
            "Chamada sem resultado virou ação nativa ou ganhou resultado inventado")
    error = next(item["output"] for item in actual if item.get("type") == "function_call_output" and item.get("call_id") == "error-call")
    require(error == "[Claude tool_result: is_error=true]\n" + MARKERS["error"], "Erro histórico perdido")
    image_urls = [part["image_url"] for item in actual for part in item.get("content", [])
                  if part.get("type") == "input_image"]
    require(len(image_urls) == 1, "Imagem ausente ou duplicada")
    image = base64.b64decode(image_urls[0].split(",", 1)[1], validate=True)
    require(image == original["image"], "Payload PNG alterado")
    tools = body.get("tools", [])
    require(isinstance(tools, list), "Catálogo de ferramentas nativas inválido")
    tools = [*tools, *(item for item in actual if item.get("type") == "additional_tools")]
    require(bool(tools), "Catálogo de ferramentas nativas ausente")

    def tool_names(value: object) -> set[str]:
        if isinstance(value, dict):
            return ({value["name"]} if isinstance(value.get("name"), str) else set()).union(
                *(tool_names(child) for child in value.values()))
        return set().union(*(tool_names(child) for child in value)) if isinstance(value, list) else set()

    names = tool_names(tools)
    require(not names.intersection({"Read", "Edit", "Bash"}), "Ferramenta Claude registrada no catálogo ativo")
    require(bool(names.intersection({"exec", "exec_command", "shell_command", "shell", "code_mode", "js"})),
            "Não foi reconhecida ferramenta nativa no catálogo capturado")
    expected_items = [without_ids(item) for item in context.items]
    require(selected == expected_items, "Ordem/conteúdo integral divergiram")
    return {"imported_item_count": len(selected), "positions": positions,
            "expected_projected_bytes": len(json_bytes(expected_items)),
            "captured_projected_bytes": len(json_bytes(selected)),
            "expected_projected_sha256": digest(json_bytes(expected_items)),
            "captured_projected_sha256": digest(json_bytes(selected)), "projected_byte_difference": 0,
            "output_chars": len(output), "output_bytes": len(output.encode()), "output_sha256": digest(output.encode()),
            "arguments_sha256": digest(json_bytes(json.loads(call["arguments"]))), "image_sha256": digest(image),
            "image_bytes": len(image), "native_tool_names": sorted(names), "request_raw_bytes": request["raw_bytes"],
            "request_raw_sha256": request["raw_sha256"],
            "comparison": "Itens integrais na ordem; IDs nativos omitidos da projeção, campos adicionais do protocolo ignorados."}


def inspect_rollout(path: Path, *, before_turn: bool) -> None:
    rows = [json.loads(line) for line in path.read_bytes().splitlines()]
    require(not any(row.get("type") == "compacted" for row in rows), "Rollout contém compactação")
    events = [row.get("payload", {}).get("type") for row in rows if row.get("type") == "event_msg"]
    require(not any(event and "compact" in event for event in events), "Rollout contém evento de compactação")
    if before_turn:
        require(not set(events).intersection({"task_started", "user_message", "agent_message", "task_complete"}),
                "Importação iniciou um turno")


async def capture_turn(home: Path, cwd: Path, prepared, server: CaptureServer,
                       notifications: list, process_results: list, *, stage: str) -> dict:
    from app.adapters.codex.appserver import AppServerClient
    from app.adapters.codex.lancador import CLIENT_INFO

    client = AppServerClient()
    process = None
    collector = None
    completed = asyncio.get_running_loop().create_future()

    async def collect() -> None:
        try:
            async for event in client.notifications():
                notifications.append({"stage": stage, **event})
                method = event.get("method", "")
                require("compact" not in method.lower(), f"Compactação notificada: {method}")
                require(not client.server_requests, "API simulada pediu ação/autoridade inesperada")
                if method == "turn/completed" and event.get("params", {}).get("threadId") == prepared.thread_id:
                    turn = event["params"]["turn"]
                    require(turn.get("status") == "completed", f"Turno não concluiu: {turn.get('status')}")
                    if not completed.done():
                        completed.set_result(turn)
            if not completed.done():
                completed.set_exception(EOFError("app-server encerrou antes de turn/completed"))
        except Exception as exc:
            if not completed.done():
                completed.set_exception(exc)
            raise

    try:
        await client.start(codex_home=home, cwd=str(cwd), tool_output_token_limit=prepared.tool_output_token_limit)
        process = client._proc
        collector = asyncio.create_task(collect())
        await client.request("initialize", {"clientInfo": CLIENT_INFO, "capabilities": {"experimentalApi": True}})
        before = len(server.requests)
        config = (await client.request("config/read", {"cwd": str(cwd), "includeLayers": False}))["config"]
        require(config.get("tool_output_token_limit") == prepared.tool_output_token_limit, "Budget de retomada divergiu")
        resumed = await client.request("thread/resume", {"threadId": prepared.thread_id})
        thread = resumed.get("thread", {})
        require(thread.get("id") == prepared.thread_id and resumed.get("model") == prepared.model,
                "Retomada mudou thread/modelo")
        require(resumed.get("modelProvider") == "local_capture", "Retomada mudou provedor")
        require(len(server.requests) == before, "Retomada iniciou inferência")
        await client.request("turn/start", {"threadId": prepared.thread_id,
                                           "input": [{"type": "text", "text": f"Continuação artificial {stage}."}]})
        await asyncio.wait_for(completed, TIMEOUT)
        require(len(server.requests) == before + 1, "Turno fez zero ou vários pedidos; não é captura simples")
        require(not server.failures, "Servidor de captura registrou falha")
        return server.requests[-1]
    finally:
        primary_error = sys.exc_info()[0]
        await client.close()
        if process is not None:
            require(process.returncode is not None, "Saída do processo não foi confirmada")
            process_results.append({"stage": stage, "pid": process.pid, "returncode": process.returncode,
                                    "exit_confirmed": True})
        if collector is not None:
            results = await asyncio.gather(collector, return_exceptions=True)
            for error in results:
                if isinstance(error, BaseException):
                    logging.error("Leitura nativa encerrada: %s", error, exc_info=error)
                    if primary_error is None:
                        raise error


async def run_probe(root: Path, binary: Path, report: dict) -> None:
    from app import codex_contas, conversation_transfer as store
    from app.claude_to_codex import convert_snapshot
    from app.adapters.codex.transfer import prepare_import, resolve_limits
    from app.conversation_transfer import ConversationSource, TransferError, TransferPhase, TransferRecord

    original_base = store._base
    store._base = lambda: root / "transfers"
    server = CaptureServer(root)
    server_thread = threading.Thread(target=server.serve_forever, name="artificial-capture", daemon=True)
    server_thread.start()
    notifications, process_results = [], []
    report["processes"] = process_results
    try:
        home = root / "codex"
        home.mkdir(mode=0o700)
        workspace = root / "workspace"
        workspace.mkdir(mode=0o700)
        catalog = home / "catalog.json"
        report["catalog"] = write_catalog(catalog)
        config = f'''model = "{MODEL}"
model_provider = "local_capture"
model_catalog_json = {json.dumps(str(catalog))}
approval_policy = "never"
sandbox_mode = "read-only"
web_search = "disabled"
[features]
plugins = false
apps = false
[model_providers.local_capture]
name = "Captura artificial local"
base_url = "http://127.0.0.1:{server.server_port}/v1"
wire_api = "responses"
requires_openai_auth = false
supports_websockets = false
'''
        (home / "config.toml").write_text(config, encoding="utf-8")
        report["config_sha256"] = digest(config.encode())
        account = codex_contas.Account("artificial-capture", home, False)
        version = await asyncio.create_subprocess_exec(str(binary), "--version", stdout=asyncio.subprocess.PIPE,
                                                       stderr=asyncio.subprocess.PIPE, cwd=workspace)
        try:
            out, err = await asyncio.wait_for(version.communicate(), TIMEOUT)
        except BaseException:
            if version.returncode is None:
                version.kill()
                await version.wait()
            raise
        (root / "version.stderr").write_bytes(err)
        report["binary_version"] = out.decode().strip()
        process_results.append({"stage": "version", "pid": version.pid, "returncode": version.returncode,
                                "exit_confirmed": version.returncode is not None})
        require(version.returncode == 0 and out.decode().strip() == f"codex-cli {VERSION}", "Versão nativa não suportada")

        def record_source(path: Path, context) -> TransferRecord:
            record = TransferRecord(str(uuid.uuid4()), "artificial-capture", "k:artificial-capture", TransferPhase.SOURCE_STOPPED,
                ConversationSource(str(path), "claude", context.source_digest, tuple(sorted(context.selected_uuids))),
                {"name": "artificial-capture", "key": "artificial-key", "cwd": str(workspace)}, None, None, None)
            store.save_transfer(record)
            # O importador registra seu dono no manifesto privado da preparação.
            store.prepare_runtime(record)
            return record

        source, original = create_source(root)
        context = convert_snapshot(source)
        require(sorted(context.selected_uuids) == original["selected_uuids"], "Ramo selecionado divergiu da fonte artificial")
        require(not context.source_limitations, "Fonte artificial não deve ter limitações")
        report["source"] = {"sha256": context.source_digest, "bytes": source.stat().st_size,
                            "output_chars": len(original["output"]), "output_bytes": len(original["output"].encode()),
                            "output_sha256": digest(original["output"].encode()),
                            "arguments_sha256": digest(json_bytes(original["arguments"])),
                            "image_bytes": len(original["image"]), "image_sha256": digest(original["image"]),
                            "selected_uuids": sorted(context.selected_uuids), "markers": {
                                key: {"bytes": len(value.encode()), "sha256": digest(value.encode())}
                                for key, value in MARKERS.items()}}
        write_json(root / "converted-items.json", context.items)
        record = record_source(source, context)
        prepared = await prepare_import(account, str(workspace), context, MODEL, None, "Ask for approval", transfer_id=record.id)
        require(not server.requests, "Importação iniciou pedido HTTP")
        require(not (home / "auth.json").exists(), "Autenticação inesperada no CODEX_HOME privado")
        inspect_rollout(Path(prepared.rollout_path), before_turn=True)
        report["import"] = {**asdict(prepared), "http_requests_before_turn": 0,
                            "exit_confirmation": "prepare_import encerra seu cliente no finally; close aguarda o processo."}
        report["captures"] = {}
        # prepare_import fecha seu app-server; o primeiro pedido já prova carga nativa de thread persistida.
        for stage in ("after_import", "after_restart"):
            request = await capture_turn(home, workspace, prepared, server, notifications, process_results, stage=stage)
            report["captures"][stage] = inspect_capture(request, context, original)
            inspect_rollout(Path(prepared.rollout_path), before_turn=False)
        require(report["captures"]["after_import"]["captured_projected_sha256"] ==
                report["captures"]["after_restart"]["captured_projected_sha256"], "Contexto mudou após restart")

        large_source, _ = create_source(root, oversized=True)
        large_context = convert_snapshot(large_source)
        large_record = record_source(large_source, large_context)
        before = len(server.requests)
        try:
            await prepare_import(account, str(workspace), large_context, MODEL, None, "Ask for approval", transfer_id=large_record.id)
        except TransferError as exc:
            require(exc.code == "session_transfer_context_budget_exceeded", f"Recusa inesperada: {exc.code}")
            rejected = store.load_transfer(large_record.id)
            require(rejected is not None and rejected.boundary is None, "Importação maior deixou fronteira concluída")
            report["oversized"] = {"error_code": exc.code, "source_bytes": large_source.stat().st_size,
                "source_sha256": large_context.source_digest, "http_requests": len(server.requests) - before,
                "limits": asdict(resolve_limits(json.loads(catalog.read_text())["models"][0], {})),
                "limitation": "Prova só a recusa do orçamento; não há Claude físico a restaurar neste roteiro."}
            if rejected.destination_meta:
                rollout = Path(rejected.destination_meta["rollout_path"])
                if rollout.exists():
                    inspect_rollout(rollout, before_turn=True)
        else:
            raise RuntimeError("Fonte maior que a margem foi aceita")
        require(len(server.requests) == before and not server.failures, "Recusa maior iniciou HTTP ou compactação")
        report["http_request_count"] = len(server.requests)
        report["compaction_requests"] = 0
        report["status"] = "captura_simulada_conferida"
    finally:
        write_json(root / "notifications.json", notifications)
        write_json(root / "http-summary.json", {"requests": server.requests, "failures": server.failures})
        server.shutdown()
        server.server_close()
        server_thread.join(timeout=5)
        require(not server_thread.is_alive(), "Servidor privado não encerrou")
        store._base = original_base


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", type=Path, required=True, help="Binário bruto Codex 0.159.3, caminho absoluto")
    parser.add_argument("--output-dir", type=Path, required=True, help="Diretório privado novo sob /tmp/claude-to-codex")
    args = parser.parse_args()
    require(args.binary.is_absolute(), "--binary deve ser absoluto")
    binary = args.binary.resolve(strict=True)
    require(binary.is_file() and os.access(binary, os.X_OK), "Binário não é arquivo executável")
    require(binary.read_bytes()[:4] == b"\x7fELF", "Use binário bruto ELF, sem wrapper interativo")
    require(args.output_dir.is_absolute(), "--output-dir deve ser absoluto")
    root = args.output_dir.resolve(strict=False)
    allowed = Path("/tmp/claude-to-codex").resolve(strict=True)
    require(root.parent == allowed, "Saída deve ser diretório novo diretamente sob /tmp/claude-to-codex")
    require(not root.exists() and not args.output_dir.is_symlink(), "Saída já existe; escolha diretório novo")
    old_umask = os.umask(0o077)
    root.mkdir(mode=0o700)
    report = {"status": "iniciada", "method": "Fonte Claude artificial → convert_snapshot → prepare_import → stdio nativo → HTTP SSE simulado",
              "binary": str(binary), "binary_sha256": digest(binary.read_bytes()), "model": MODEL,
              "limitations": ["Sem inferência real ou autenticação", "Tokens de resposta são simulados, não medidos",
                              "Imagem confere bytes; não prova compreensão multimodal", "TUI/WebSocket/Hangar/Windows pendentes",
                              "Primeira captura ocorre após prepare_import fechar o processo de preparação"]}
    original_env = dict(os.environ)
    original_cwd = Path.cwd()
    try:
        private_home = root / "home"
        private_home.mkdir(mode=0o700)
        private_bin = root / "bin"
        private_bin.mkdir(mode=0o700)
        (private_bin / "codex").symlink_to(binary)
        environment = {"PATH": str(private_bin) + os.pathsep + os.defpath, "HOME": str(private_home),
                       "USERPROFILE": str(private_home), "CODEX_HOME": str(root / "codex"),
                       "XDG_CONFIG_HOME": str(private_home / ".config"), "XDG_DATA_HOME": str(private_home / ".local/share"),
                       "XDG_STATE_HOME": str(private_home / ".local/state"), "XDG_CACHE_HOME": str(private_home / ".cache"),
                       "TMPDIR": str(root), "LANG": "C.UTF-8", "NO_PROXY": "127.0.0.1,localhost"}
        os.environ.clear()
        os.environ.update(environment)
        logging.basicConfig(filename=root / "product.log", level=logging.INFO, encoding="utf-8")
        checkout = Path(__file__).resolve().parents[1]
        sys.path.insert(0, str(checkout / "backend"))
        # Settings procura .env no cwd; as importações também precisam de raiz privada.
        os.chdir(root)

        def interrupted(signum, frame) -> None:
            raise KeyboardInterrupt(f"Captura interrompida por sinal {signum}")

        signal.signal(signal.SIGTERM, interrupted)
        asyncio.run(run_probe(root, binary, report))
    except BaseException as exc:
        report["status"] = "falhou"
        report["error_type"] = type(exc).__name__
        report["error_code"] = getattr(exc, "code", None)
        (root / "failure.txt").write_text(traceback.format_exc(), encoding="utf-8")
        chain, seen, error = [], set(), exc
        while error is not None and id(error) not in seen:
            seen.add(id(error))
            chain.append({"type": type(error).__name__, "message": str(error),
                          "traceback": "".join(traceback.format_exception(error))})
            error = error.__cause__ or error.__context__
        write_json(root / "failure-chain.json", chain)
        print(f"Captura falhou: {type(exc).__name__}; diagnóstico privado em {root / 'failure.txt'}")
    finally:
        write_json(root / "results.json", report)
        os.environ.clear()
        os.environ.update(original_env)
        os.chdir(original_cwd)
        os.umask(old_umask)
    print(f"Relatório privado: {root / 'results.json'}")
    return 0 if report["status"] == "captura_simulada_conferida" else 1


if __name__ == "__main__":
    raise SystemExit(main())
