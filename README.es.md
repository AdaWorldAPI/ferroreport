# ferroreport

**La forma más rápida de generar reportes de JasperReports.** ferroreport lee las plantillas
`.jrxml` que ya tienes y las convierte a PDF, Excel (XLSX) o CSV en milisegundos, sin JVM: unas
**300 veces más rápido** que JasperReports en reportes del día a día, con **la décima parte de la
memoria**, PDF **idénticos píxel por píxel**, hojas de cálculo con **las mismas celdas** que saca
Jasper y plantillas que **no pueden ejecutar código** en tu servidor.

<p align="center"><a href="https://cristianencarnacion.github.io/ferroreport/"><img src="docs/race.gif" alt="Carrera: JasperReports genera 1 factura en 3.01 s; ferroreport genera 301 en el mismo tiempo" width="760"></a></p>

**[Pruébalo en tu navegador](https://cristianencarnacion.github.io/ferroreport/)** ·
[Descargas](https://github.com/cristianEncarnacion/ferroreport/releases) ·
[Docker](https://github.com/cristianEncarnacion/ferroreport/pkgs/container/ferroreport)

> Estado: **vista previa.** Las 28 plantillas comparables de la batería de pruebas salen píxel por
> píxel igual que en JasperReports 6.20, incluyendo bandas divididas entre páginas, imágenes,
> subreportes, tablas, listas, crosstabs, códigos QR y de barras, texto con formato y varias columnas; los gráficos siguen de cerca el
> estilo de JFreeChart. Las 30 también exportan el mismo Excel y CSV que Jasper. Todavía faltan
> algunas funciones; ver
> [compatibilidad](README.md#compatibility). No está afiliado a Jaspersoft ni a Cloud Software Group.

## Por qué

- **Sin JVM.** Un binario de ~8 MB (con las fuentes incluidas) que arranca en milisegundos.
  Importa en CLIs, funciones serverless y contenedores.
- **Salida idéntica.** Cada fixture se renderiza con los dos motores y se compara píxel por
  píxel: hoy los 28 comparables salen idénticos. Sus exportaciones a Excel y CSV se comparan
  celda por celda, y las 30 coinciden.
- **PDF, Excel y CSV.** La misma plantilla da un PDF, un `.xlsx` armado como el de JasperReports
  (celdas combinadas, fuentes, rellenos, bordes, alturas de fila, imágenes y gráficos) o un CSV.
- **Seguro con plantillas ajenas.** JasperReports compila las expresiones a Java y carga
  archivos `.jasper` serializados; así ocurrieron fallos recientes de ejecución remota de código
  (CVE-2025-10492, CVE-2026-6009). ferroreport nunca deserializa objetos e interpreta las
  expresiones con una lista cerrada de operaciones: sin reflexión, sin carga de clases, sin
  acceso a la red. Las imágenes y subreportes solo se leen de las carpetas de recursos de la
  plantilla.
- **Úsalo desde cualquier lenguaje.** Paquetes para JavaScript (Node.js, Deno, Bun y
  navegadores) y Python, librería de Rust, CLI, y una API HTTP en imagen Docker para todo lo
  demás (Java, C#, PHP, Go, Ruby...).

## Desde tu lenguaje

> **[Documentación completa](https://cristianencarnacion.github.io/ferroreport/docs.html)** (en inglés): cada comando,
> endpoint, función y opción. El [paquete de npm](https://www.npmjs.com/package/ferroreport), los
> [binarios de Releases](https://github.com/cristianEncarnacion/ferroreport/releases) y la imagen Docker ya están disponibles.
> Los [crates](https://crates.io/crates/ferroreport) también; el paquete de PyPI se está publicando.

- **JavaScript/TypeScript:** `npm install ferroreport` y
  `render(plantilla, filas, parametros, { locale: "es_DO" })` devuelve el PDF; con
  `{ format: "xlsx" }` o `"csv"`, el Excel o el CSV.
- **Python:** `pip install ferroreport` y
  `ferroreport.render("factura.jrxml", filas, parametros, locale="es_DO")` (agrega
  `format="xlsx"` o `"csv"`, o usa `render_to_file(..., "factura.xlsx", ...)`).
- **Cualquier lenguaje por HTTP:**
  `docker run -p 8080:8080 -v "$PWD/plantillas:/templates" -e FERROREPORT_TEMPLATES=/templates ghcr.io/cristianencarnacion/ferroreport`
  y luego `POST /render/factura` con `{"data": [...], "params": {...}}` te devuelve el PDF
  (agrega `"format": "xlsx"` o `"csv"` para Excel o CSV).
- **Rust:** `cargo add ferroreport`. **Línea de comandos:** binarios en
  [Releases](https://github.com/cristianEncarnacion/ferroreport/releases); `-d -` y `-o -` leen
  las filas de stdin y escriben el PDF en stdout.

## Inicio rápido

```bash
cargo install --path crates/ferroreport-cli

ferroreport render factura.jrxml --data filas.json --params parametros.json -o factura.pdf --locale es_DO
ferroreport render factura.jrxml --data filas.json -o factura.xlsx   # Excel (o .csv)
ferroreport doctor factura.jrxml     # qué soporta (y qué no) esta plantilla
```

`filas.json` es un arreglo JSON de objetos cuyas claves coinciden con los `<field>` de la
plantilla. Los valores se convierten a la clase que declara cada campo (`BigDecimal`, fechas
ISO-8601 a `java.util.Date`, arreglos anidados a `java.util.List`, base64 a `byte[]`, etc.).

Las imágenes y subreportes se buscan junto a la plantilla (agrega más carpetas con
`--resources CARPETA`). Un subreporte que apunta a `lineas.jasper` se renderiza desde
`lineas.jrxml`: nunca se cargan archivos `.jasper` compilados. Los arreglos JSON anidados
alimentan subreportes, por ejemplo `new JRMapCollectionDataSource($F{lineas})` o
`((JsonDataSource)$P{REPORT_DATA_SOURCE}).subDataSource("lineas")`.

## Qué soporta

- Bandas completas, incluida la división de bandas y frames entre páginas (`splitType`).
- Texto estático y dinámico, líneas, rectángulos, elipses, frames, imágenes PNG/JPEG,
  subreportes, y códigos de los componentes barcode4j: QR, Code 128, Code 39 (con ASCII completo),
  EAN-13, EAN-8, UPC-A, Interleaved 2 of 5 y Codabar. Cada barra se verifica contra barcode4j.
- Estilos con herencia y estilos condicionales, texto justificado, `isRemoveLineWhenBlank`,
  `isPrintRepeatedValues`, grupos, variables y tiempos de evaluación.
- Los QR se codifican igual que ZXing (misma versión, máscara y módulos) y se dibujan como los
  dibuja JasperReports: sirven para facturas electrónicas impresas, como el sello del e-CF
  dominicano.
- Reportes de varias columnas (orden vertical).
- Texto con formato (markup `styled` y `html`): negritas, cursivas, fuentes, tamaños, colores,
  resaltado, superíndices, saltos de línea y listas, también cuando el texto sigue en otra página.
- Texto rotado (izquierda, derecha, de cabeza).
- Gráficos: pastel (también 3D), barras (también 3D y apiladas), líneas, áreas, XY y dispersión, con
  títulos, leyendas y datasets por registro, grupo o reporte. Siguen el estilo de JFreeChart, aunque
  no son idénticos píxel por píxel.
- Tablas (`jr:table`) como las crea el asistente de Jaspersoft Studio: grupos de columnas, encabezados
  y pies de tabla, de columna y de grupo, columnas ocultas, celda sin datos, y encabezados que se
  repiten en cada página.
- Listas (`jr:list`) verticales, también cuando siguen en otra página.
- Exportación a Excel (XLSX) y CSV como la de JasperReports con su configuración por defecto: todas
  las páginas en una hoja con el nombre del reporte, cada texto en las celdas que cubre
  (combinadas) con su fuente, colores, bordes y alineación; líneas como bordes, imágenes sobre sus
  celdas, y gráficos y códigos de barras como imágenes (al doble de resolución que Jasper). Los
  valores van como texto formateado, igual que en Jasper; con `--xlsx-numbers` los números van
  como números con su formato.
- Crosstabs (tablas cruzadas): grupos de filas y columnas con totales al inicio o al final,
  varias medidas (suma, conteo, promedio, máximo, mínimo...), totales con `$V{medida_grupo_ALL}`,
  parámetros, datos del reporte o de un subdataset. Si no caben a lo ancho se parten en bloques de
  columnas, y si no caben a lo alto siguen en la página siguiente con las cabeceras repetidas.
- Aún no: `percentageOf` en crosstabs, listas horizontales, orden horizontal de columnas,
  conexiones JDBC.
  La tabla completa está en el [README en inglés](README.md#compatibility).

## Rendimiento

Render completo por línea de comandos (desde que arranca el proceso hasta el PDF en disco),
mediana de 5 corridas en una VM Linux de 2 vCPU:

| Fixture | JasperReports 6.20 | ferroreport | Más rápido | Memoria pico |
|---|---|---|---|---|
| 02-invoice (2 páginas) | 3.01 s | 0.010 s | 309× | 194 MiB → 16 MiB |
| 03-grouped-sales (4 páginas) | 3.22 s | 0.013 s | 253× | 205 MiB → 16 MiB |
| 09-large (20,000 filas, 477 páginas) | 7.22 s | 0.39 s | 19× | 489 MiB → 52 MiB |

En reportes pequeños, la mayor parte del tiempo de Jasper es el arranque de la JVM y la
compilación del JRXML, que un servidor de larga vida amortiza. Contando solo sus fases de
llenado y exportación, ferroreport es unas 85× más rápido en los reportes pequeños y 13× en el
grande. Además sus PDF pesan menos (1.1 MB en vez de 4.5 MB en 09-large).

## Roadmap

- **Fase 2 (en curso):** listo: división de bandas, imágenes, subreportes, estilos
  condicionales, texto justificado, códigos QR y de barras, multicolumna, texto con formato, texto rotado, gráficos
  tablas, listas y crosstabs. Siguiente: memoria constante en reportes enormes.
- **Fase 3:** paquetes para npm y PyPI, versión WebAssembly con playground en el navegador y
  vista previa en vivo para VS Code.
- **Fase 4:** más tipos de gráficos; listo: exportación a Excel (XLSX) y CSV.

La documentación completa (compatibilidad, cómo se verifica la salida, cómo contribuir) está en
el [README en inglés](README.md).

## Licencia

MIT o Apache-2.0, a tu elección. Las fuentes DejaVu incluidas tienen su propia licencia.
JasperReports y Jaspersoft son marcas de Cloud Software Group, Inc.; este proyecto es
independiente.
