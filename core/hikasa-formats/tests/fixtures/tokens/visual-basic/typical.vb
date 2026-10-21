Imports System.Text

Public Class Greeter
    ' Builds a greeting.
    Public Function Greet(ByVal name As String) As String
        Dim sb As New StringBuilder()
        sb.Append("Hello, ").Append(name)
        If name.Length > 10 Then
            sb.Append("!")
        End If
        Return sb.ToString()
    End Function
End Class
