use bundle_licenses_lib::bundle::BundleBuilder;
use std::fmt::Write;

fn main() {
	let bundle = BundleBuilder::new().exec().expect("Could not create bundle");
	let mut licenses_file = String::new();
	for l in bundle.third_party_libraries() {
		writeln!(licenses_file, "# {} v{} ({})", l.package_name, l.package_version, l.repository).unwrap();
		for license in &l.licenses {
			writeln!(licenses_file, "```\n{}\n```", license.text).unwrap();
		}
	}
	std::fs::write("./public/LICENSES.md", licenses_file).expect("Could not write licenses file");
}