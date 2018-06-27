#!/usr/bin/env bash

MANDIR="./man"
mkdir -p "${MANDIR}"
OXYCMD="oxy"
OXYOUT="$("${OXYCMD}" help)"
OXYAUTHOR="$(head -n2 <<< "${OXYOUT}" | tail -n1)"
OXYSUB=($(awk '
	$1=="" {now=0}
	now {print$1}
	$1=="SUBCOMMANDS:" {now=1}
' <<< "${OXYOUT}"))
OXYOPT=($(awk '
	$1=="" {now=0}
	now {print$1}
	$1=="OPTIONS:" {now=1}
' <<< "${OXYOUT}" | cut -c2))
OXYOPTSTRING="$(tr -d ' ' <<< "${OXYOPT[@]}")"
OXYNAME="$(head -n1 <<< "${OXYOUT}" | awk '{print$1}')"
OXYVER="$(head -n1 <<< "${OXYOUT}" | awk '{print$2}')"

exec > "${MANDIR}/${OXYNAME}.1"

printf -- '.TH "%s" 1 "%s" "version %s"\n' "${OXYNAME}" "$(TZ='UTC' date)" "${OXYVER}"

printf -- '.SH NAME\n'
printf -- '%s\n' "${OXYNAME}"

printf -- '.SH SYNOPSIS\n'
printf -- '%s' "${OXYNAME}"
if [[ "${OXYOPTSTRING}" != "" ]]
then printf -- ' [-%s]' "${OXYOPTSTRING}"
fi
printf -- ' <%s>\n' "$(tr ' ' '|' <<< "${OXYSUB[@]}")"

printf -- '.SH "SEE ALSO"\n'
printf -- '.B oxy-%s(1)\n.PP\n' "${OXYSUB[@]}"

printf -- '.SH AUTHOR\n'
printf -- '%s\n' "${OXYAUTHOR}"

for COMMAND in "${OXYSUB[@]}"; do
	SUBOUT="$("${OXYCMD}" help "${COMMAND}")"
	SUBOPT=($(awk '
		$1=="" {now=0}
		now && $1 ~ "^-.," {print$1}
		$1=="OPTIONS:" {now=1}
	' <<< "${SUBOUT}" | cut -c2))
	SUBARGS=($(awk '
		$1=="" {now=0}
		now {print$1}
		$1=="ARGS:" {now=1}
	' <<< "${SUBOUT}"))
	SUBNAME="$(head -n1 <<< "${SUBOUT}" | awk '{print$1}')"
	SUBOPTSTRING="$(tr -d ' ' <<< "${SUBOPT[@]}")"

	exec > "${MANDIR}/${SUBNAME}.1"

	printf -- '.TH "%s" 1 "%s" "version %s"\n' "${SUBNAME}" "$(TZ='UTC' date)" "${OXYVER}"

	printf -- '.SH NAME\n'
	printf -- '%s\n' "${SUBNAME}"

	printf -- '.SH SYNOPSIS\n'
	printf -- '%s %s' "${OXYNAME}" "$(head -n1 <<< "${SUBOUT}" | awk '{print$1}' | cut -d'-' -f2-)"
	if [[ "${SUBOPTSTRING}" != "" ]]
	then printf -- ' [-%s]' "${SUBOPTSTRING}"
	fi
	printf -- ' %s\n' "${SUBARGS[*]}"

	printf -- '.SH DESCRIPTION\n'
	printf -- '%s\n.PP\n' "$(head -n2 <<< "${SUBOUT}" | tail -n1)"
	awk '
		now && $1=="" {now=0}
		!now && has && $1 ~ ":$" {exit}
		now {print}
		$1=="OPTIONS:" {now=1;has=1}
	' <<< "${SUBOUT}" |
		sed -re 's/^[[:space:]]*//' |
		sed -re 's/^([[:space:]]*[[:alpha:]].*)/\n.RS\n\1\n.RE\n/' |
		sed -re 's/[[:space:]]{2,}(.*)/\n.RS\n\1\n.RE\n/gm' |
		sed -re 's/^([[:space:]]*-.*)/.B \1/' |
		sed 'N;/^\n$/d;P;D'
	printf -- '\n'

	printf -- '.SH AUTHOR\n'
	printf -- '%s\n' "${OXYAUTHOR}"
done
