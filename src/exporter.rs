// src/exporter.rs
// Exporta el grafo posicional de ProxyIA como SVG interactivo.
// MEJORADO v2.2:
//   - Tooltips con path completo al hacer hover
//   - Enlaces <a> para abrir archivos (file:// protocol)
//   - Tema oscuro/claro automático (prefers-color-scheme)
//   - Círculos con radio según número de dependencias
//   - Animación sutil al cargar

use anyhow::Result;
use crate::db::DatabaseManager;
use crate::positional::Position3D;

/// Escala las coordenadas al rango [min, max] preservando proporciones.
fn scale_range(values: &[f64], min: f64, max: f64) -> Vec<f64> {
    if values.is_empty() {
        return vec![];
    }
    let vmin = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let vmax = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = vmax - vmin;
    if range < 0.0001 {
        return values.iter().map(|_| (min + max) / 2.0).collect();
    }
    values.iter().map(|v| min + (v - vmin) / range * (max - min)).collect()
}

/// Cuenta cuántas dependencias tiene cada nodo (como source o target)
fn count_dep_degrees(ids: &[i64], dependencies: &[(i64, i64)]) -> Vec<usize> {
    let mut degrees = vec![0usize; ids.len()];
    for (src, dst) in dependencies {
        if let Some(idx) = ids.iter().position(|i| i == src) {
            degrees[idx] += 1;
        }
        if let Some(idx) = ids.iter().position(|i| i == dst) {
            degrees[idx] += 1;
        }
    }
    degrees
}

/// Genera un SVG interactivo con el grafo posicional del proyecto.
pub async fn export_project_svg(
    db: &DatabaseManager,
    project_id: i64,
) -> Result<String> {
    let nodes = db.get_all_nodes(project_id).await?;
    let dependencies = db.get_project_dependencies(project_id).await?;

    if nodes.is_empty() {
        return Ok("<svg viewBox=\"0 0 400 300\" xmlns=\"http://www.w3.org/2000/svg\">\n\
            <text x=\"10\" y=\"20\" font-family=\"monospace\">Sin nodos en el proyecto.</text>\n\
            </svg>".to_string());
    }

    // Extraer posiciones
    let mut ids: Vec<i64> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut full_paths: Vec<String> = Vec::new();
    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    let mut zs: Vec<f64> = Vec::new();

    for (id, path_str, pos_bytes) in &nodes {
        if let Some(pos) = Position3D::from_bytes(pos_bytes) {
            ids.push(*id);
            let name = std::path::Path::new(path_str)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "root".to_string());
            names.push(name);
            full_paths.push(path_str.clone());
            xs.push(pos.x as f64);
            ys.push(pos.y as f64);
            zs.push(pos.z as f64);
        }
    }

    // Escalar coordenadas a viewBox 800x600, con margen 40
    let margin = 40.0;
    let w = 800.0;
    let h = 600.0;
    let sx = scale_range(&xs, margin, w - margin);
    let sy = scale_range(&ys, margin, h - margin);

    // Contar grados de dependencia para radio de nodos
    let degrees = count_dep_degrees(&ids, &dependencies);
    let max_deg = degrees.iter().cloned().max().unwrap_or(1).max(1);

    // Normalizar Z a [0.2, 1.0] para opacidad
    let max_z = zs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let min_z = zs.iter().cloned().fold(f64::INFINITY, f64::min);
    let z_range = if (max_z - min_z).abs() < 0.0001 { 1.0 } else { max_z - min_z };
    let opacity: Vec<f64> = zs.iter().map(|z| {
        0.3 + 0.7 * (z - min_z) / z_range
    }).collect();

    // Construir aristas
    let mut edges_x1 = Vec::new();
    let mut edges_y1 = Vec::new();
    let mut edges_x2 = Vec::new();
    let mut edges_y2 = Vec::new();

    for (src, dst) in &dependencies {
        if let (Some(ix1), Some(ix2)) = (ids.iter().position(|i| i == src), ids.iter().position(|i| i == dst)) {
            edges_x1.push(sx[ix1]);
            edges_y1.push(sy[ix1]);
            edges_x2.push(sx[ix2]);
            edges_y2.push(sy[ix2]);
        }
    }

    // Generar SVG interactivo
    let mut svg = String::from(
        r#"<svg viewBox="0 0 800 600" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">
<style>
  @media (prefers-color-scheme: dark) {
    .bg { fill: #1a1a2e; }
    .edge { stroke: #4a4a6a; stroke-width: 1.2; stroke-opacity: 0.5; }
    .node { fill: #6c63ff; stroke: #8b83ff; stroke-width: 1.5; }
    .label { font-family: monospace; font-size: 9px; fill: #e0e0e0; }
    .title { font-family: sans-serif; font-size: 16px; fill: #ffffff; }
    .legend-text { font-family: monospace; font-size: 10px; fill: #ccc; }
  }
  @media (prefers-color-scheme: light) {
    .bg { fill: #ffffff; }
    .edge { stroke: #999; stroke-width: 1.2; stroke-opacity: 0.6; }
    .node { fill: #4a90d9; stroke: #2c5f8a; stroke-width: 1.5; }
    .label { font-family: monospace; font-size: 9px; fill: #333; }
    .title { font-family: sans-serif; font-size: 16px; fill: #111; }
    .legend-text { font-family: monospace; font-size: 10px; fill: #555; }
  }
  .edge { transition: stroke-opacity 0.3s; }
  .node { transition: fill-opacity 0.3s, r 0.2s; cursor: pointer; }
  .node:hover { fill-opacity: 1.0 !important; stroke-width: 2.5; }
  .label { pointer-events: none; user-select: none; }
  @keyframes fadeIn { from { opacity: 0; } to { opacity: 1; } }
  .fade-in { animation: fadeIn 0.5s ease-in; }
</style>
<rect class="bg" width="800" height="600" rx="8" />
"#);

    // Título
    svg.push_str(&format!(
        "<text class=\"title\" x=\"10\" y=\"25\">🗺️ ProxyIA - Grafo Estructural</text>\n"
    ));

    // Aristas
    svg.push_str("<g class=\"fade-in\">\n");
    for i in 0..edges_x1.len() {
        svg.push_str(&format!(
            "<line class=\"edge\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" />\n",
            edges_x1[i], edges_y1[i], edges_x2[i], edges_y2[i]
        ));
    }
    svg.push_str("</g>\n");

    // Nodos (circulares, interactivos con tooltip + enlace)
    svg.push_str("<g class=\"fade-in\">\n");
    for i in 0..ids.len() {
        let r = 6.0 + (degrees[i] as f64 / max_deg as f64) * 10.0;
        let r = r.max(6.0).min(18.0); // min 6, max 18
        let tooltip_text = format!(
            "{} | {} dependencias | Z: {:.2}", full_paths[i], degrees[i], zs[i]
        );

        // Archivo local URL (file://)
        let file_url = format!("file:///{}", full_paths[i].replace(" ", "%20"));

        svg.push_str(&format!(
            r#"<a xlink:href="{}" target="_blank">
  <g>
    <circle class="node" cx="{:.1}" cy="{:.1}" r="{:.1}" fill-opacity="{:.2}" />
    <title>{}</title>
  </g>
</a>
"#,
            file_url, sx[i], sy[i], r, opacity[i], tooltip_text
        ));

        // Texto truncado si es muy largo
        let label = if names[i].len() > 20 {
            format!("{}...", &names[i][..17])
        } else {
            names[i].clone()
        };
        svg.push_str(&format!(
            "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" dominant-baseline=\"central\">{}</text>\n",
            sx[i], sy[i], label
        ));
    }
    svg.push_str("</g>\n");

    // Leyenda
    svg.push_str("<g class=\"fade-in\">\n");
    svg.push_str("<line x1=\"620\" y1=\"550\" x2=\"650\" y2=\"550\" class=\"edge\" />");
    svg.push_str("<text x=\"655\" y=\"555\" class=\"legend-text\">Dependencia</text>\n");

    svg.push_str(&format!(
        "<circle cx=\"620\" cy=\"570\" r=\"6\" class=\"node\" fill-opacity=\"0.3\" /><text x=\"632\" y=\"575\" class=\"legend-text\">Baja Z (profundo)</text>\n"
    ));
    svg.push_str(&format!(
        "<circle cx=\"620\" cy=\"585\" r=\"6\" class=\"node\" fill-opacity=\"1.0\" /><text x=\"632\" y=\"590\" class=\"legend-text\">Alta Z (superficial)</text>\n"
    ));

    // Créditos
    svg.push_str(&format!(
        "<text x=\"790\" y=\"595\" text-anchor=\"end\" font-family=\"monospace\" font-size=\"8\" fill=\"#888\">ProxyIA v2.2</text>\n"
    ));
    svg.push_str("</g>\n");
    svg.push_str("</svg>\n");
    Ok(svg)
}