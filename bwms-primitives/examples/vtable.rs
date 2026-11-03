// Lê N entradas (u64 cruas, com os bits de fixup) de um endereço de dado em cada binário.
fn ler(bin: &[u8], vm: u64, n: usize) -> Vec<u64> {
    let m = bwms_primitives::enderecos::fatia_arm64(bin).unwrap();
    let s = bwms_primitives::enderecos::secoes(m).unwrap();
    let sec = s.iter().find(|x| vm >= x.vm && vm < x.vm + x.tamanho).unwrap();
    let o = (sec.arquivo + (vm - sec.vm)) as usize;
    println!("  {vm:#x} em {},{}", sec.segmento, sec.nome);
    (0..n).map(|k| u64::from_le_bytes(m[o + k * 8..o + k * 8 + 8].try_into().unwrap())).collect()
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bin = std::fs::read(&a[1]).unwrap();
    for v in &a[2..] {
        let vm = u64::from_str_radix(v.trim_start_matches("0x"), 16).unwrap();
        let e = ler(&bin, vm, 30);
        println!("  {:x?}", e.iter().map(|x| x & 0xF_FFFF_FFFF).collect::<Vec<_>>());
    }
}
