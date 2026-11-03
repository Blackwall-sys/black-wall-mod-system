fn main(){
  for p in ["base\\worlds\\03_night_city\\_compiled\\default\\always_loaded_0.streamingsector",
            "base\\\\worlds\\\\03_night_city\\\\_compiled\\\\default\\\\always_loaded_0.streamingsector"] {
    println!("{} -> {:#x} ({})", if p.contains("\\\\"){"duplo"}else{"simples"}, bwms_primitives::resource_path_hash(p), bwms_primitives::resource_path_hash(p));
  }
}
