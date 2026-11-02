use super::fbx_tree::*;
use super::*;
use std::io::Write;

fn tree() -> Vec<FNode> {
    let mut root = FNode::new("Objects");
    root.leaf("Verts", Prop::ArrD(vec![0.5, -1.0, 2.25]));
    root.leaf("Ids", Prop::ArrI(vec![0, 1, -3]));
    root.leaf("Name", Prop::Str("a\"b".to_string()));
    root.add(FNode::new("Child").with(Prop::I64(7)).with(Prop::F64(1.5)));
    vec![root]
}

#[test]
fn binary_tree_round_trip() {
    let bytes = write_binary(&tree(), 7400);
    let (nodes, v) = parse_binary(&bytes).unwrap();
    assert_eq!(v, 7400);
    assert_eq!(nodes.len(), 1);
    let o = &nodes[0];
    assert_eq!(o.child("Verts").unwrap().props[0].vec_f64().unwrap(), vec![0.5, -1.0, 2.25]);
    assert_eq!(o.child("Ids").unwrap().props[0].vec_i64().unwrap(), vec![0, 1, -3]);
    assert_eq!(o.child("Child").unwrap().props[0].i64(), Some(7));
}

#[test]
fn binary_wide_headers() {
    let bytes = write_binary(&tree(), 7500);
    let (nodes, _) = parse_binary(&bytes).unwrap();
    assert_eq!(nodes[0].child("Child").unwrap().props[1].f64(), Some(1.5));
}

#[test]
fn ascii_tree_round_trip() {
    let text = write_ascii(&tree(), 7400);
    let nodes = parse_ascii(&text).unwrap();
    let o = &nodes[0];
    assert_eq!(o.child("Verts").unwrap().props[0].vec_f64().unwrap(), vec![0.5, -1.0, 2.25]);
    assert_eq!(o.child("Ids").unwrap().props[0].vec_i64().unwrap(), vec![0, 1, -3]);
    assert_eq!(o.child("Name").unwrap().str_at(0), "a\"b");
}

#[test]
fn compressed_arrays_are_inflated() {
    // hand built node with one deflate-encoded double array
    let vals = [1.0f64, 2.0, 3.0, 4.0];
    let raw: Vec<u8> = vals.iter().flat_map(|v| v.to_le_bytes()).collect();
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(&raw).unwrap();
    let comp = enc.finish().unwrap();
    let mut prop = vec![b'd'];
    prop.extend_from_slice(&4u32.to_le_bytes());
    prop.extend_from_slice(&1u32.to_le_bytes());
    prop.extend_from_slice(&(comp.len() as u32).to_le_bytes());
    prop.extend_from_slice(&comp);
    let mut f = Vec::new();
    f.extend_from_slice(BINARY_MAGIC);
    f.extend_from_slice(&[0x1A, 0]);
    f.extend_from_slice(&7400u32.to_le_bytes());
    let end = (f.len() + 12 + 1 + 1 + prop.len()) as u32;
    f.extend_from_slice(&end.to_le_bytes());
    f.extend_from_slice(&1u32.to_le_bytes());
    f.extend_from_slice(&(prop.len() as u32).to_le_bytes());
    f.push(1);
    f.push(b'V');
    f.extend_from_slice(&prop);
    f.extend_from_slice(&[0u8; 13]);
    let (nodes, _) = parse_binary(&f).unwrap();
    assert_eq!(nodes[0].props[0].vec_f64().unwrap(), vals.to_vec());
}

#[test]
fn truncated_binary_is_an_error() {
    let bytes = write_binary(&tree(), 7400);
    assert!(parse_binary(&bytes[..40]).is_err());
}

#[test]
fn obj_quads_negative_indices_and_materials() {
    let dir = std::env::temp_dir().join(format!("aztec_obj_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("m.mtl"), "newmtl a\nKd 1 0 0\nnewmtl b\nKd 0 1 0\nmap_Kd tex.png\n").unwrap();
    let obj = "mtllib m.mtl\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl a\nf -4/-4 -3/-3 -2/-2 -1/-1\nusemtl b\nf 1 2 3\n";
    let io = obj::parse(obj, &dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(io.nodes.len(), 1, "one object with a material per face");
    assert_eq!(io.nodes[0].face_material, vec![Some(0), Some(1)]);
    assert_eq!(io.nodes[0].mesh.faces[0].len(), 4);
    assert_eq!(io.nodes[0].mesh.uvs[2], crate::math::Vec2::new(1.0, 1.0));
    assert_eq!(io.materials.len(), 2);
    assert_eq!(io.materials[0].color, [1.0, 0.0, 0.0]);
    assert!(io.materials[1].texture.as_deref().unwrap().ends_with("tex.png"));
}

#[test]
fn obj_without_faces_is_rejected() {
    assert!(obj::parse("v 0 0 0\n", std::path::Path::new(".")).is_err());
}

fn sample_scene() -> Scene {
    let mut s = Scene::new();
    let (t, _) = s.create_mesh_object("polyCube", vec![], Some("box")).unwrap();
    s.set_attr(t, "translateX", 3.0).unwrap();
    s.set_attr(t, "rotateY", 45.0).unwrap();
    s.set_attr(t, "scaleZ", 2.0).unwrap();
    let m = s.create_material(Some("blue"));
    s.set_attr(m, "colorB", 1.0).unwrap();
    s.assign_material(m, t).unwrap();
    s.set_keyframe(t, "translateY", Some(1.0), Some(0.0), Interp::Linear, 0).unwrap();
    s.set_keyframe(t, "translateY", Some(12.0), Some(4.0), Interp::Linear, 0).unwrap();
    s
}

#[test]
fn fbx_scene_round_trip_binary_and_ascii() {
    for ascii in [false, true] {
        let mut src = sample_scene();
        let io = export_scene(&mut src, false).unwrap();
        let tree = fbx::build_tree(&io);
        let bytes = if ascii { write_ascii(&tree, 7400).into_bytes() } else { write_binary(&tree, 7400) };
        let back = fbx::from_bytes(&bytes, std::path::Path::new(".")).unwrap();
        assert_eq!(back.nodes.len(), 1);
        let n = &back.nodes[0];
        assert_eq!(n.kind, IoKind::Mesh);
        assert!((n.t.x - 3.0).abs() < 1e-5 && (n.r.y - 45.0).abs() < 1e-4 && (n.s.z - 2.0).abs() < 1e-5);
        assert_eq!(back.materials.len(), 1);
        assert!((back.materials[0].color[2] - 1.0).abs() < 1e-6);
        assert_eq!(back.curves.len(), 1);
        assert_eq!(back.curves[0].attr, "translateY");
        assert_eq!(back.curves[0].keys.len(), 2);
        assert!((back.curves[0].keys[1].0 - 12.0).abs() < 1e-3 && (back.curves[0].keys[1].1 - 4.0).abs() < 1e-6);
        let mut dst = Scene::new();
        let r = import_into(&mut dst, &back);
        assert_eq!((r.meshes, r.materials, r.curves), (1, 1, 1));
        assert_eq!(r.faces, 6, "split corners are welded back into a cube");
    }
}

#[test]
fn obj_scene_round_trip_bakes_world_transform() {
    let mut src = sample_scene();
    let io = export_scene(&mut src, false).unwrap();
    let (text, mtl) = obj::to_text(&io, "x.obj");
    assert!(mtl.unwrap().contains("newmtl blue"));
    let back = obj::parse(&text, std::path::Path::new(".")).unwrap();
    let mut dst = Scene::new();
    let r = import_into(&mut dst, &back);
    assert_eq!((r.meshes, r.faces), (1, 6));
    let (lo, hi) = match dst.node(dst.mesh_shapes_under(r.top[0])[0]).map(|n| &n.data) {
        Some(NodeData::Mesh(md)) => md.mesh.bounds(),
        _ => panic!("no mesh"),
    };
    // a unit cube scaled 2 along z, turned 45 degrees about y and moved 3 along x
    assert!((hi.x - lo.x - 2.0 * 0.7071068 - 0.7071068).abs() < 1e-3, "{:?} {:?}", lo, hi);
    assert!((lo.x + hi.x - 6.0).abs() < 1e-3);
}

#[test]
fn selection_export_needs_a_selection() {
    let mut s = sample_scene();
    s.selection.clear();
    assert!(export_scene(&mut s, true).is_err());
}

#[test]
fn old_fbx_versions_are_rejected() {
    let nodes = vec![FNode::new("Objects")];
    let bytes = write_binary(&nodes, 6100);
    assert!(fbx::from_bytes(&bytes, std::path::Path::new(".")).unwrap_err().contains("not supported"));
}

fn two_material_scene() -> Scene {
    let mut s = Scene::new();
    let (t, sh) = s.create_mesh_object("polyCube", vec![], Some("box")).unwrap();
    let a = s.create_material(Some("matA"));
    let b = s.create_material(Some("matB"));
    s.set_attr(b, "colorG", 1.0).unwrap();
    s.assign_material(a, t).unwrap();
    // select the shape's mesh as custom so faces can be addressed
    if let Some(NodeData::Mesh(md)) = s.nodes.get_mut(sh).map(|n| &mut n.data) {
        let _ = md;
    }
    s.delete_history(sh);
    s.assign_faces(sh, &[0, 1], Some(b), None).unwrap();
    s
}

#[test]
fn per_face_materials_round_trip_through_obj() {
    let mut src = two_material_scene();
    let io = export_scene(&mut src, false).unwrap();
    assert_eq!(io.materials.len(), 2);
    let (text, mtl) = obj::to_text(&io, "x.obj");
    assert_eq!(text.matches("usemtl").count(), 2, "{}", text);
    let mtl = mtl.unwrap();
    assert!(mtl.contains("newmtl matA") && mtl.contains("newmtl matB"));
    let dir = std::env::temp_dir().join(format!("aztec_zone_obj_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("x.mtl"), &mtl).unwrap();
    let back = obj::parse(&text, &dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(back.nodes.len(), 1);
    let mut dst = Scene::new();
    let r = import_into(&mut dst, &back);
    assert_eq!((r.meshes, r.faces), (1, 6));
    let shape = dst.mesh_shapes_under(r.top[0])[0];
    let fz = dst.face_zones(shape).unwrap();
    assert_eq!(fz.iter().filter(|z| **z != 0).count().min(fz.iter().filter(|z| **z == 0).count()), 2, "two faces differ from the base material: {:?}", fz);
}

#[test]
fn per_face_materials_round_trip_through_fbx() {
    for ascii in [false, true] {
        let mut src = two_material_scene();
        let io = export_scene(&mut src, false).unwrap();
        let tree = fbx::build_tree(&io);
        let bytes = if ascii { write_ascii(&tree, 7400).into_bytes() } else { write_binary(&tree, 7400) };
        let back = fbx::from_bytes(&bytes, std::path::Path::new(".")).unwrap();
        assert_eq!(back.nodes.len(), 1, "no extra part objects any more");
        assert_eq!(back.materials.len(), 2);
        let used: std::collections::HashSet<usize> = back.nodes[0].face_material.iter().flatten().copied().collect();
        assert_eq!(used.len(), 2);
        let mut dst = Scene::new();
        let r = import_into(&mut dst, &back);
        let shape = dst.mesh_shapes_under(r.top[0])[0];
        let fz = dst.face_zones(shape).unwrap();
        assert_eq!(fz.len(), 6);
        assert!(fz.iter().any(|z| *z != 0));
    }
}

const MA_SAMPLE: &str = r#"//Maya ASCII 2009 scene
//Name: sample.ma
requires maya "2009";
currentUnit -l centimeter -a degree -t film;
fileInfo "application" "maya";
createNode transform -n "group1";
	setAttr ".t" -type "double3" 10 0 0 ;
	setAttr ".r" -type "double3" 0 90 0 ;
createNode transform -n "pCube1" -p "group1";
	setAttr ".t" -type "double3" 1 2 3 ;
	setAttr ".s" -type "double3" 2 2 2 ;
	setAttr ".rp" -type "double3" 0 0.5 0 ;
createNode mesh -n "pCubeShape1" -p "pCube1";
	setAttr -k off ".v";
	setAttr ".vir" yes;
	setAttr ".uvst[0].uvsn" -type "string" "map1";
	setAttr -s 8 ".vt[0:7]"  -0.5 -0.5 0.5 0.5 -0.5 0.5 -0.5 0.5 0.5 0.5 0.5 0.5
		 -0.5 0.5 -0.5 0.5 0.5 -0.5 -0.5 -0.5 -0.5 0.5 -0.5 -0.5;
	setAttr -s 12 ".ed[0:11]"  0 1 0 2 3 0 4 5 0 6 7 0 0 2 0 1 3 0 2 4 0 3 5 0
		 4 6 0 5 7 0 6 0 0 7 1 0;
	setAttr -s 6 -ch 24 ".fc[0:5]" -type "polyFaces" 
		f 4 0 5 -2 -5
		mu 0 4 0 1 3 2
		f 4 1 7 -3 -7
		mu 0 4 2 3 5 4
		f 4 2 9 -4 -9
		mu 0 4 4 5 7 6
		f 4 3 11 -1 -11
		mu 0 4 6 7 9 8
		f 4 -12 -10 -8 -6
		mu 0 4 1 10 11 3
		f 4 10 4 6 8
		mu 0 4 12 0 2 13;
createNode joint -n "joint1";
	setAttr ".t" -type "double3" 0 1 0 ;
	setAttr ".jo" -type "double3" 0 0 90 ;
createNode joint -n "joint2" -p "joint1";
	setAttr ".t" -type "double3" 0 3 0 ;
createNode lambert -n "lambert2";
	setAttr ".c" -type "float3" 1 0.25 0.5 ;
createNode shadingEngine -n "lambert2SG";
createNode file -n "file1";
	setAttr ".ftn" -type "string" "sourceimages/wall.png";
createNode animCurveTL -n "pCube1_translateX";
	setAttr ".tan" 18;
	setAttr -s 3 ".ktv[0:2]"  1 0 12 5 24 0;
createNode animCurveTA -n "pCube1_rotateY";
	setAttr -s 2 ".ktv[0:1]"  1 0 24 90;
connectAttr "lambert2.oc" "lambert2SG.ss";
connectAttr "pCubeShape1.iog" "lambert2SG.dsm" -na;
connectAttr "file1.oc" "lambert2.c";
connectAttr "pCube1_translateX.o" "pCube1.tx";
connectAttr "pCube1_rotateY.o" "pCube1.ry";
//End of sample.ma
"#;

#[test]
fn maya_ascii_hierarchy_geometry_materials_and_animation() {
    let io = mayaascii::parse(MA_SAMPLE).unwrap();
    let names: Vec<&str> = io.nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["group1", "pCube1", "joint1", "joint2"]);
    let cube = &io.nodes[1];
    assert_eq!(cube.parent, Some(0));
    assert_eq!(cube.kind, IoKind::Mesh);
    assert_eq!((cube.mesh.positions.len(), cube.mesh.faces.len()), (8, 6));
    assert!(cube.mesh.faces.iter().all(|f| f.len() == 4));
    assert_eq!((cube.t.x, cube.t.y, cube.t.z), (1.0, 2.0, 3.0));
    assert_eq!(cube.s.x, 2.0);
    assert_eq!(cube.rp.y, 0.5);
    assert_eq!(io.nodes[0].r.y, 90.0);
    assert_eq!(io.fps, 24.0);
    // the shader reached the shape through its shading group, with the file texture on its color
    assert_eq!(io.materials.len(), 1);
    assert_eq!(io.materials[0].name, "lambert2");
    assert_eq!(io.materials[0].color, [1.0, 0.25, 0.5]);
    assert_eq!(io.materials[0].texture.as_deref(), Some("sourceimages/wall.png"));
    assert_eq!(cube.material, Some(0));
    // joints: the orient is folded into the rotation of the static joint
    assert_eq!(io.nodes[2].kind, IoKind::Joint);
    assert!((io.nodes[2].r.z - 90.0).abs() < 1e-3, "{:?}", io.nodes[2].r);
    assert_eq!(io.nodes[3].parent, Some(2));
    // animation curves
    assert_eq!(io.curves.len(), 2);
    let tx = io.curves.iter().find(|c| c.attr == "translateX").unwrap();
    assert_eq!(tx.keys.iter().map(|k| (k.0, k.1)).collect::<Vec<_>>(), vec![(1.0, 0.0), (12.0, 5.0), (24.0, 0.0)]);
    assert_eq!(tx.node, 1);
}

#[test]
fn maya_ascii_imports_into_a_scene_and_the_cube_has_the_right_size() {
    let io = mayaascii::parse(MA_SAMPLE).unwrap();
    let mut scene = Scene::new();
    let r = import_into(&mut scene, &io);
    assert_eq!((r.meshes, r.faces, r.joints, r.materials, r.curves), (1, 6, 2, 1, 2));
    let mut s = crate::session::Session::new();
    s.scene = scene;
    let o = crate::mel::execute_script(&mut s, "xform -q -bb pCube1;");
    let bb: Vec<f32> = o.lines[0].split_whitespace().map(|v| v.parse().unwrap()).collect();
    // scaled by 2 around the pivot the cube is 2 units wide in the group's rotated space: rotated 90 degrees about Y
    assert!(((bb[3] - bb[0]) - 2.0).abs() < 1e-3 && ((bb[5] - bb[2]) - 2.0).abs() < 1e-3, "{:?}", bb);
}

#[test]
fn maya_poly_primitives_without_explicit_geometry() {
    let src = r#"requires maya "2009";
createNode transform -n "pSphere1";
createNode mesh -n "pSphereShape1" -p "pSphere1";
createNode polySphere -n "polySphere1";
	setAttr ".r" 2;
	setAttr ".sa" 12;
connectAttr "polySphere1.out" "pSphereShape1.i";
createNode transform -n "pLoft1";
createNode mesh -n "pLoftShape1" -p "pLoft1";
	setAttr -k off ".v";
"#;
    let io = mayaascii::parse(src).unwrap();
    assert_eq!(io.nodes[0].kind, IoKind::Mesh);
    let (lo, hi) = io.nodes[0].mesh.bounds();
    assert!((hi.x - lo.x - 4.0).abs() < 0.2, "radius 2: {:?} {:?}", lo, hi);
    // a mesh whose geometry comes from history we cannot evaluate is reported, not guessed
    assert_eq!(io.nodes[1].kind, IoKind::Group);
    assert!(io.warnings.iter().any(|w| w.contains("pLoftShape1")), "{:?}", io.warnings);
}

#[test]
fn maya_ascii_rejects_binary_and_non_maya_files() {
    assert!(mayaascii::parse("hello world").is_err());
    let dir = std::env::temp_dir().join(format!("aztec_ma_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mb = dir.join("scene.ma");
    std::fs::write(&mb, b"FOR4\0\0\0\0Maya").unwrap();
    assert!(mayaascii::read(mb.to_str().unwrap()).unwrap_err().contains("binary"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn maya_ascii_opens_through_the_file_command() {
    let dir = std::env::temp_dir().join(format!("aztec_ma_open_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("lesson.ma");
    std::fs::write(&f, MA_SAMPLE).unwrap();
    let mut s = crate::session::Session::new();
    crate::mel::execute_script(&mut s, "polyCube -name old;");
    let o = crate::mel::execute_script(&mut s, &format!("file -open \"{}\"; ls -type transform;", f.display()));
    assert!(o.error.is_none(), "{:?}", o.error);
    assert!(o.lines.iter().any(|l| l == "pCube1") && !o.lines.iter().any(|l| l == "old"), "{:?}", o.lines);
    let _ = std::fs::remove_dir_all(&dir);
    // exporting to .ma is not offered
    let e = crate::mel::execute_script(&mut s, "file -export \"x.ma\"").error.unwrap();
    assert!(e.contains("imported but not exported"), "{}", e);
}

#[test]
fn pivots_round_trip_through_fbx() {
    let mut src = Scene::new();
    let (t, _) = src.create_mesh_object("polyCube", vec![], Some("box")).unwrap();
    src.set_attr(t, "rotatePivotY", 1.5).unwrap();
    src.set_attr(t, "scalePivotY", 1.5).unwrap();
    src.set_attr(t, "rotateZ", 30.0).unwrap();
    let io = export_scene(&mut src, false).unwrap();
    assert_eq!(io.nodes[0].rp.y, 1.5);
    let tree = fbx::build_tree(&io);
    let bytes = write_binary(&tree, 7400);
    let back = fbx::from_bytes(&bytes, std::path::Path::new(".")).unwrap();
    assert!((back.nodes[0].rp.y - 1.5).abs() < 1e-5 && (back.nodes[0].sp.y - 1.5).abs() < 1e-5, "{:?}", back.nodes[0]);
    assert!((back.nodes[0].r.z - 30.0).abs() < 1e-4);
    let m0 = io.nodes[0].local_matrix();
    let m1 = back.nodes[0].local_matrix();
    for c in 0..4 {
        for r in 0..4 {
            assert!((m0.m[c][r] - m1.m[c][r]).abs() < 1e-4);
        }
    }
}
