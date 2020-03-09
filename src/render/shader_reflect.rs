use crate::render::render_graph::{
    BindGroup, BindType, Binding, TextureViewDimension, UniformProperty, UniformPropertyType,
};
use spirv_reflect::{
    types::{
        ReflectDescriptorBinding, ReflectDescriptorSet, ReflectDescriptorType, ReflectDimension,
        ReflectTypeDescription, ReflectTypeFlags,
    },
    ShaderModule,
};
use zerocopy::AsBytes;
// use rspirv::{binary::Parser, dr::Loader, lift::LiftContext};

// TODO: use rspirv when structured representation is ready. this way we can remove spirv_reflect, which is a non-rust dependency
// pub fn get_shader_layout(spirv_data: &[u32]) {
//     let mut loader = Loader::new();  // You can use your own consumer here.
//     {
//         let p = Parser::new(spirv_data.as_bytes(), &mut loader);
//         p.parse().unwrap();
//     }
//     let module = loader.module();
//     let structured = LiftContext::convert(&module).unwrap();
//     println!("{:?}", structured.types);
// }

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ShaderLayout {
    pub bind_groups: Vec<BindGroup>,
    pub entry_point: String,
}

pub fn get_shader_layout(spirv_data: &[u32]) -> ShaderLayout {
    match ShaderModule::load_u8_data(spirv_data.as_bytes()) {
        Ok(ref mut module) => {
            let entry_point_name = module.get_entry_point_name();
            let mut bind_groups = Vec::new();
            for descriptor_set in module.enumerate_descriptor_sets(None).unwrap() {
                let bind_group = reflect_bind_group(&descriptor_set);
                bind_groups.push(bind_group);
            }

            ShaderLayout {
                bind_groups,
                entry_point: entry_point_name,
            }
        }
        Err(err) => panic!("Failed to reflect shader layout: {:?}", err),
    }
}

fn reflect_bind_group(descriptor_set: &ReflectDescriptorSet) -> BindGroup {
    let mut bindings = Vec::new();
    for descriptor_binding in descriptor_set.bindings.iter() {
        let binding = reflect_binding(descriptor_binding);
        bindings.push(binding);
    }

    BindGroup::new(descriptor_set.set, bindings)
}

fn reflect_dimension(type_description: &ReflectTypeDescription) -> TextureViewDimension {
    match type_description.traits.image.dim {
        ReflectDimension::Type1d => TextureViewDimension::D1,
        ReflectDimension::Type2d => TextureViewDimension::D2,
        ReflectDimension::Type3d => TextureViewDimension::D3,
        ReflectDimension::Cube => TextureViewDimension::Cube,
        dimension => panic!("unsupported image dimension: {:?}", dimension),
    }
}

fn reflect_binding(binding: &ReflectDescriptorBinding) -> Binding {
    let type_description = binding.type_description.as_ref().unwrap();
    let (name, bind_type) = match binding.descriptor_type {
        ReflectDescriptorType::UniformBuffer => (
            &type_description.type_name,
            BindType::Uniform {
                dynamic: false,
                properties: vec![reflect_uniform(type_description)],
            },
        ),
        ReflectDescriptorType::SampledImage => (
            &binding.name,
            BindType::SampledTexture {
                dimension: reflect_dimension(type_description),
                multisampled: false,
            },
        ),
        ReflectDescriptorType::Sampler => (&binding.name, BindType::Sampler),
        _ => panic!("unsupported bind type {:?}", binding.descriptor_type),
    };

    Binding {
        index: binding.binding,
        bind_type,
        name: name.to_string(),
    }
}

#[derive(Debug)]
enum NumberType {
    Int,
    UInt,
    Float,
}

fn reflect_uniform(type_description: &ReflectTypeDescription) -> UniformProperty {
    let uniform_property_type = if type_description
        .type_flags
        .contains(ReflectTypeFlags::STRUCT)
    {
        reflect_uniform_struct(type_description)
    } else {
        reflect_uniform_numeric(type_description)
    };

    UniformProperty {
        name: type_description.type_name.to_string(),
        property_type: uniform_property_type,
    }
}

fn reflect_uniform_struct(type_description: &ReflectTypeDescription) -> UniformPropertyType {
    let mut properties = Vec::new();
    for member in type_description.members.iter() {
        properties.push(reflect_uniform(member));
    }

    UniformPropertyType::Struct(properties)
}

fn reflect_uniform_numeric(type_description: &ReflectTypeDescription) -> UniformPropertyType {
    let traits = &type_description.traits;
    let number_type = if type_description.type_flags.contains(ReflectTypeFlags::INT) {
        match traits.numeric.scalar.signedness {
            0 => NumberType::UInt,
            1 => NumberType::Int,
            signedness => panic!("unexpected signedness {}", signedness),
        }
    } else if type_description
        .type_flags
        .contains(ReflectTypeFlags::FLOAT)
    {
        NumberType::Float
    } else {
        panic!("unexpected type flag {:?}", type_description.type_flags);
    };

    // TODO: handle scalar width here

    if type_description
        .type_flags
        .contains(ReflectTypeFlags::MATRIX)
    {
        match (
            number_type,
            traits.numeric.matrix.column_count,
            traits.numeric.matrix.row_count,
        ) {
            (NumberType::Float, 3, 3) => UniformPropertyType::Mat3,
            (NumberType::Float, 4, 4) => UniformPropertyType::Mat4,
            (number_type, column_count, row_count) => panic!(
                "unexpected uniform property matrix format {:?} {}x{}",
                number_type, column_count, row_count
            ),
        }
    } else {
        match (number_type, traits.numeric.vector.component_count) {
            (NumberType::Int, 1) => UniformPropertyType::Int,
            (NumberType::Float, 3) => UniformPropertyType::Vec3,
            (NumberType::Float, 4) => UniformPropertyType::Vec4,
            (NumberType::UInt, 4) => UniformPropertyType::UVec4,
            (number_type, component_count) => panic!(
                "unexpected uniform property format {:?} {}",
                number_type, component_count
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{
        render_graph::{BindGroup, BindType, Binding, UniformProperty, UniformPropertyType},
        Shader, ShaderStage,
    };

    #[test]
    fn test_reflection() {
        let vertex_shader = Shader::from_glsl(
            ShaderStage::Vertex,
            r#"
            #version 450
            layout(location = 0) in vec4 a_Pos;
            layout(location = 0) out vec4 v_Position;
            layout(set = 0, binding = 0) uniform Camera {
                mat4 ViewProj;
            };
            layout(set = 1, binding = 0) uniform texture2D Texture;

            void main() {
                v_Position = a_Pos;
                gl_Position = ViewProj * v_Position;
            }
        "#,
        )
        .get_spirv_shader(None);

        let layout = vertex_shader.reflect_layout().unwrap();
        assert_eq!(
            layout,
            ShaderLayout {
                entry_point: "main".to_string(),
                bind_groups: vec![
                    BindGroup::new(
                        0,
                        vec![Binding {
                            index: 0,
                            name: "Camera".to_string(),
                            bind_type: BindType::Uniform {
                                dynamic: false,
                                properties: vec![UniformProperty {
                                    name: "Camera".to_string(),
                                    property_type: UniformPropertyType::Struct(vec![
                                        UniformProperty {
                                            name: "".to_string(),
                                            property_type: UniformPropertyType::Mat4,
                                        }
                                    ]),
                                }],
                            },
                        }]
                    ),
                    BindGroup::new(
                        1,
                        vec![Binding {
                            index: 0,
                            name: "Texture".to_string(),
                            bind_type: BindType::SampledTexture {
                                multisampled: false,
                                dimension: TextureViewDimension::D2,
                            },
                        }]
                    ),
                ]
            }
        );
    }
}