// Compiler types the generated bindings use that netstandard2.0 lacks.

namespace System.Runtime.CompilerServices
{
    internal static class IsExternalInit { }
}

namespace System.Diagnostics.CodeAnalysis
{
    [AttributeUsage(AttributeTargets.Parameter)]
    internal sealed class NotNullWhenAttribute(bool returnValue) : Attribute
    {
        public bool ReturnValue { get; } = returnValue;
    }
}
