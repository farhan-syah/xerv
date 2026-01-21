{{/*
Expand the name of the chart.
*/}}
{{- define "xerv.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "xerv.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "xerv.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "xerv.labels" -}}
helm.sh/chart: {{ include "xerv.chart" . }}
{{ include "xerv.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "xerv.selectorLabels" -}}
app.kubernetes.io/name: {{ include "xerv.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "xerv.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "xerv.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Determine if we should use StatefulSet (for Raft) or Deployment (for other backends)
*/}}
{{- define "xerv.useStatefulSet" -}}
{{- if eq .Values.dispatch.backend "raft" }}
{{- true }}
{{- else }}
{{- false }}
{{- end }}
{{- end }}

{{/*
Image name with tag
*/}}
{{- define "xerv.image" -}}
{{- $tag := default .Chart.AppVersion .Values.image.tag }}
{{- printf "%s:%s" .Values.image.repository $tag }}
{{- end }}

{{/*
Headless service name (for Raft peer discovery)
*/}}
{{- define "xerv.headlessServiceName" -}}
{{- printf "%s-headless" (include "xerv.fullname" .) }}
{{- end }}

{{/*
Service mesh pod annotations
Returns the appropriate annotations for Istio or Linkerd based on configuration
*/}}
{{- define "xerv.serviceMeshAnnotations" -}}
{{- if .Values.serviceMesh.enabled }}
{{- if eq .Values.serviceMesh.provider "istio" }}
{{- if .Values.serviceMesh.istio.injection.enabled }}
sidecar.istio.io/inject: "true"
{{- end }}
{{- else if eq .Values.serviceMesh.provider "linkerd" }}
{{- if .Values.serviceMesh.linkerd.injection.enabled }}
linkerd.io/inject: enabled
{{- end }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Service mesh pod labels
Returns additional labels for service mesh integration
*/}}
{{- define "xerv.serviceMeshLabels" -}}
{{- if .Values.serviceMesh.enabled }}
{{- if eq .Values.serviceMesh.provider "istio" }}
app: {{ include "xerv.name" . }}
version: {{ .Chart.AppVersion | default "v1" | quote }}
{{- else if eq .Values.serviceMesh.provider "linkerd" }}
app: {{ include "xerv.name" . }}
{{- end }}
{{- end }}
{{- end }}

{{/*
gRPC service name (for service mesh routing)
*/}}
{{- define "xerv.grpcServiceName" -}}
{{- printf "%s-grpc" (include "xerv.fullname" .) }}
{{- end }}
