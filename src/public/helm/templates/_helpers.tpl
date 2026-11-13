{{/*
    Chart name.
*/}}
{{- define "aka-public.name" -}}
{{- .Chart.Name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
    Image pull secret (rolodex/parkinglot convention): rendered in-chart from
    registryCredentials values, so deploys stay `--set`-driven.
*/}}
{{- define "aka-public.imagePullSecret" }}
{{- printf "{\"auths\": {\"%s\": {\"auth\": \"%s\"}}}" .Values.registryCredentials.registry (printf "%s:%s" .Values.registryCredentials.username .Values.registryCredentials.password | b64enc) | b64enc }}
{{- end }}
